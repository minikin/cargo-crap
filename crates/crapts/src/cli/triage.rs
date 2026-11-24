#[cfg(feature = "triage")]
use super::InputError;
use super::{Result, scoring::warn, settings::TriageSettings};
use crate::{FileAnalysis, duplicates::Scan};
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(feature = "triage")]
mod cache;
#[cfg(feature = "triage")]
mod source;

pub(super) fn judge(
    scan: Option<&mut Scan>,
    settings: &TriageSettings,
    files: &[FileAnalysis],
    sources: &HashMap<PathBuf, Vec<u8>>,
    boundary: &Path,
    err: &mut dyn Write,
) -> Result<()> {
    let Some(scan) = scan.filter(|_| settings.enabled.value) else {
        return Ok(());
    };
    #[cfg(feature = "triage")]
    {
        assess(scan, settings, files, sources, boundary, err)
    }
    #[cfg(not(feature = "triage"))]
    {
        let _ = (scan, files, sources, boundary);
        warn(
            err,
            "[duplicates.triage] is enabled, but this crapts was built without the triage client. Use @minikin/crapts or cargo install crapts --features triage.",
        )
    }
}

#[cfg(feature = "triage")]
fn assess(
    scan: &mut Scan,
    settings: &TriageSettings,
    files: &[FileAnalysis],
    snapshots: &HashMap<PathBuf, Vec<u8>>,
    boundary: &Path,
    err: &mut dyn Write,
) -> Result<()> {
    use cargo_crap_core::duplicates::triage::{
        Settings, TriageError, budget, provider::by_id, run_with_cache,
    };
    let asked: Vec<_> = scan.comparison.as_ref().map_or_else(
        || (0..scan.groups.len()).collect(),
        |c| c.reported().collect(),
    );
    if asked.is_empty() {
        return Ok(());
    }
    let pairs: Vec<_> = asked
        .iter()
        .map(|&i| scan.groups[i].representative.clone())
        .collect();
    let provider = by_id(settings.provider.value)
        .ok_or_else(|| InputError::message("unknown duplicate triage provider"))?;
    let mut resolved = Settings::for_cache(provider, &settings.model.value, None, |name| {
        std::env::var(name).ok()
    });
    resolved.max_requests = settings.max_requests.value;
    let cache = match cache::resolve(
        std::env::consts::OS,
        boundary,
        |name| std::env::var_os(name),
        std::time::SystemTime::now(),
    ) {
        Ok(cache) => Some(cache),
        Err(error) => {
            warn(err, &format!("could not use the triage cache ({error})"))?;
            None
        },
    };
    let mut cache_warning = None;
    let result = source::Sources::new(files, snapshots)
        .map_err(TriageError::Source)
        .and_then(|sources| {
            run_with_cache(
                &pairs,
                &resolved,
                &source::QUESTIONS,
                cache.as_ref(),
                |message| cache_warning = Some(message),
                |pair| sources.pair(pair, provider.id(), &resolved.model),
            )
        });
    if let Some(message) = cache_warning {
        warn(err, &message)?;
    }
    match result {
        Ok(triaged) => {
            if let Some(warning) = resolved
                .max_requests
                .and_then(|limit| budget::warning(limit, triaged.skipped))
            {
                writeln!(err, "{warning}").map_err(|e| InputError::at(Path::new("stderr"), e))?;
            }
            let mut assessments = vec![None; scan.groups.len()];
            for (index, verdict) in asked.into_iter().zip(triaged.verdicts) {
                assessments[index] =
                    verdict.map(|raw| raw.assessment(settings.confidence_floor.value));
            }
            scan.judgments = Some(crate::duplicates::Judgments {
                assessments,
                skipped: triaged.skipped,
                provider: provider.display_name(),
                model: resolved.model,
            });
        },
        Err(error) => warn(
            err,
            &format!("duplicate triage skipped: {error}; the duplicates are reported untriaged"),
        )?,
    }
    Ok(())
}
