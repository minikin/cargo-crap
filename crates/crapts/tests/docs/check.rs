use crate::fixture::{Repo, Run, VERSION};
use crate::markers::{Marker, Stream};
use crate::normalize::Normalizer;
use crate::pages::{self, Page, Sample};

const FIXTURE_PATHS: [&str; 6] = [
    "src/cart.ts",
    "src/ops.ts",
    "coverage/lcov.info",
    "coverage/lcov.data",
    "coverage/coverage-final.json",
    "crapts-baseline.json",
];

pub fn normalizer() -> Normalizer<'static> {
    Normalizer {
        windows: cfg!(windows),
        version: VERSION,
        fixture_paths: &FIXTURE_PATHS,
    }
}

fn json_stream(marker: &Marker) -> bool {
    let Marker::Run { stream, args, .. } = marker else {
        return false;
    };
    let json = ["json", "sarif", "shields"];
    *stream == Stream::Stdout
        && (args.iter().any(|a| a == "--print-config")
            || marker
                .effective_format()
                .is_some_and(|f| json.contains(&f.as_str())))
}

pub fn check_sample(
    sample: &Sample,
    run: &Run,
) -> Result<(), String> {
    let expected = i32::from(sample.marker.exit());
    if run.code != Some(expected) {
        return Err(format!(
            "{}: expected exit {expected}, actual exit {}\nstderr: {}",
            sample.label(),
            run.code.map_or("none".to_owned(), |c| c.to_string()),
            run.stderr
        ));
    }
    let captured = match sample.marker.stream() {
        Stream::Stdout => &run.stdout,
        Stream::Stderr => &run.stderr,
    };
    let actual = normalizer().apply(captured, json_stream(&sample.marker));
    match pages::first_difference(&sample.block, &actual) {
        None => Ok(()),
        Some(difference) => Err(format!("{}: {difference}", sample.label())),
    }
}

pub fn check_page(
    page: &Page,
    repo: &Repo,
) -> Result<Vec<Sample>, String> {
    pages::literal_versions(page, VERSION)?;
    let samples = pages::samples(page)?;
    for sample in &samples {
        check_sample(sample, &repo.crapts(&sample.marker.args()))?;
    }
    Ok(samples)
}

pub fn check_pages(
    pages: &[Page],
    repo: &Repo,
) -> Result<Vec<Sample>, String> {
    let mut checked = Vec::new();
    for page in pages {
        checked.extend(check_page(page, repo)?);
    }
    let outputs = checked
        .iter()
        .filter(|s| {
            matches!(
                s.marker,
                Marker::Run {
                    stream: Stream::Stdout,
                    ..
                }
            )
        })
        .count();
    if outputs == 0 {
        return Err("no crapts-output marker under docs/crapts".to_owned());
    }
    Ok(checked)
}

pub fn check_command(
    page: &Page,
    line: usize,
    text: &str,
    repo: &Repo,
) -> Result<usize, String> {
    let Some(args) =
        pages::crapts_arguments(text).map_err(|e| format!("{}: {e}", page.at(line)))?
    else {
        return Ok(0);
    };
    let runs = pages::command_runs(&args);
    for args in &runs {
        let run = repo.crapts(args);
        if run.code != Some(0) {
            return Err(format!(
                "{}: `{text}` run as {args:?} exited {:?}: {}",
                page.at(line),
                run.code,
                run.stderr.trim()
            ));
        }
    }
    Ok(runs.len())
}

pub fn check_commands(
    pages: &[Page],
    repo: &Repo,
) -> Result<usize, String> {
    let mut runs = 0;
    for page in pages {
        for (line, text) in pages::command_lines(page) {
            runs += check_command(page, line, &text, repo)?;
        }
    }
    Ok(runs)
}

pub fn check_links(pages: &[Page]) -> Result<usize, String> {
    let mut count = 0;
    for page in pages {
        for (line, target) in pages::links(page) {
            pages::check_link(page, line, &target)?;
            count += 1;
        }
    }
    Ok(count)
}
