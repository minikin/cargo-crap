use crate::fixture::VERSION;
use crate::pages::{self, Page};
use std::collections::{BTreeMap, BTreeSet};

pub const ACTION_GUIDE: &str = "docs/crapts/guides/github-action.md";
pub const ACTION_USES: &str = "minikin/cargo-crap/actions/crapts@crapts-v1";
pub const MIN_NODE: u32 = 24;

pub struct ActionYml {
    pub inputs: BTreeMap<String, Option<String>>,
    pub outputs: BTreeSet<String>,
}

fn unquote_yaml(value: &str) -> String {
    let value = value.trim();
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner.to_owned();
        }
    }
    value.to_owned()
}

pub fn read_action(text: &str) -> ActionYml {
    let mut inputs = BTreeMap::new();
    let mut outputs = BTreeSet::new();
    let mut section = "";
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            section = line.trim_end_matches(':');
            current = None;
            continue;
        }
        let Some(key) = line.strip_prefix("  ").filter(|k| !k.starts_with(' ')) else {
            if let (Some(name), Some(default)) = (&current, line.trim().strip_prefix("default:")) {
                inputs.insert(name.clone(), Some(unquote_yaml(default)));
            }
            continue;
        };
        let name = key.trim_end_matches(':').to_owned();
        match section {
            "inputs" => {
                inputs.entry(name.clone()).or_insert(None);
                current = Some(name);
            },
            "outputs" => {
                outputs.insert(name);
            },
            _ => {},
        }
    }
    ActionYml { inputs, outputs }
}

pub fn default_cell(default: Option<&str>) -> String {
    match default {
        None | Some("") => "none".to_owned(),
        Some(v) if v == VERSION => "<version>".to_owned(),
        Some(v) => v.to_owned(),
    }
}

fn table_names(
    page: &Page,
    heading: &str,
) -> Result<Vec<(usize, String, Vec<String>)>, String> {
    let section = pages::section(page, heading)?;
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for (line, cells) in pages::table_rows(&section) {
        let name = pages::unquote(&cells[0]).to_owned();
        if !seen.insert(name.clone()) {
            return Err(format!("{}: {name} has a second row", page.at(line)));
        }
        rows.push((line, name, cells));
    }
    Ok(rows)
}

pub fn check_tables(
    page: &Page,
    action: &ActionYml,
) -> Result<(), String> {
    let inputs = table_names(page, "Inputs")?;
    for (line, name, cells) in &inputs {
        let Some(default) = action.inputs.get(name) else {
            return Err(format!(
                "{}: {name} is not an input of the Action",
                page.at(*line)
            ));
        };
        let stated = cells.get(1).map_or_default(|c| pages::unquote(c));
        let expected = default_cell(default.as_deref());
        if stated != expected {
            return Err(format!(
                "{}: the default of {name} reads {stated}, action.yml says {expected}",
                page.at(*line)
            ));
        }
    }
    let listed: BTreeSet<String> = inputs.into_iter().map(|(_, n, _)| n).collect();
    let declared: BTreeSet<String> = action.inputs.keys().cloned().collect();
    if let Some(missing) = declared.difference(&listed).next() {
        return Err(format!("{}: the inputs table lacks {missing}", page.path));
    }
    let outputs: BTreeSet<String> = table_names(page, "Outputs")?
        .into_iter()
        .map(|(_, n, _)| n)
        .collect();
    if outputs == action.outputs {
        Ok(())
    } else {
        Err(format!(
            "{}: the outputs table lists {outputs:?}, action.yml declares {:?}",
            page.path, action.outputs
        ))
    }
}

#[derive(Debug)]
pub struct Node {
    pub line: usize,
    pub key: String,
    pub value: String,
    pub children: Vec<Node>,
}

struct Raw {
    line: usize,
    indent: usize,
    text: String,
}

pub type Failure = (usize, String);

pub fn parse_yaml(
    body: &str,
    first_line: usize,
) -> Result<Vec<Node>, Failure> {
    let mut raws: Vec<Raw> = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .map(|(i, l)| Raw {
            line: first_line + i,
            indent: l.len() - l.trim_start().len(),
            text: l.trim().to_owned(),
        })
        .collect();
    let mut at = 0;
    let nodes = parse_block(&mut raws, &mut at, 0)?;
    match raws.get(at) {
        None => Ok(nodes),
        Some(raw) => Err((raw.line, "unexpected indentation".to_owned())),
    }
}

fn outside(
    line: usize,
    text: &str,
) -> Failure {
    (line, format!("`{text}` is outside the workflow subset"))
}

fn scalar(
    line: usize,
    value: &str,
) -> Result<String, Failure> {
    let unclosed = value.starts_with('[') && !value.ends_with(']');
    if unclosed || value.starts_with(['{', '&', '*', '!']) {
        return Err(outside(line, value));
    }
    Ok(unquote_yaml(value))
}

fn parse_block(
    raws: &mut [Raw],
    at: &mut usize,
    indent: usize,
) -> Result<Vec<Node>, Failure> {
    let mut nodes = Vec::new();
    while let Some(raw) = raws.get(*at) {
        if raw.indent < indent {
            break;
        }
        if raw.indent > indent {
            return Err((raw.line, "unexpected indentation".to_owned()));
        }
        let line = raw.line;
        if let Some(rest) = raw.text.strip_prefix("- ").map(str::to_owned) {
            if !rest.contains(": ") && !rest.ends_with(':') {
                *at += 1;
                nodes.push(Node {
                    line,
                    key: "-".to_owned(),
                    value: scalar(line, &rest)?,
                    children: Vec::new(),
                });
                continue;
            }
            raws[*at].indent = indent + 2;
            raws[*at].text = rest;
            let children = parse_block(raws, at, indent + 2)?;
            nodes.push(Node {
                line,
                key: "-".to_owned(),
                value: String::new(),
                children,
            });
            continue;
        }
        let (key, value) = raw
            .text
            .split_once(':')
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .ok_or_else(|| outside(line, &raw.text))?;
        *at += 1;
        let children = if value.is_empty() {
            match raws.get(*at) {
                Some(next) if next.indent > indent => {
                    let child = next.indent;
                    parse_block(raws, at, child)?
                },
                _ => Vec::new(),
            }
        } else {
            if ["|", ">", "|-", ">-"].contains(&value.as_str()) {
                while raws.get(*at).is_some_and(|r| r.indent > indent) {
                    *at += 1;
                }
            }
            Vec::new()
        };
        nodes.push(Node {
            line,
            key,
            value: scalar(line, &value)?,
            children,
        });
    }
    Ok(nodes)
}

pub struct Step {
    pub uses: Option<String>,
    pub with: Vec<(usize, String, String)>,
}

pub struct Job {
    pub line: usize,
    pub permissions: Option<BTreeMap<String, String>>,
    pub steps: Vec<Step>,
}

pub struct Workflow {
    pub line: usize,
    pub triggers: Vec<String>,
    pub permissions: Option<BTreeMap<String, String>>,
    pub jobs: Vec<Job>,
}

fn child<'a>(
    nodes: &'a [Node],
    key: &str,
) -> Option<&'a Node> {
    nodes.iter().find(|n| n.key == key)
}

fn flat_scalar(node: &Node) -> Result<String, Failure> {
    if node.children.is_empty() && !node.value.starts_with('[') {
        Ok(node.value.clone())
    } else {
        Err((node.line, format!("`{}` must be a scalar", node.key)))
    }
}

fn triggers(node: &Node) -> Result<Vec<String>, Failure> {
    if !node.children.is_empty() {
        return Ok(node.children.iter().map(|c| c.key.clone()).collect());
    }
    let raw = node.value.trim();
    if raw.starts_with('[') || raw.ends_with(']') {
        let Some(inner) = raw.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
            return Err(outside(node.line, &format!("on: {}", node.value)));
        };
        if inner.contains(['[', ']']) {
            return Err(outside(node.line, &format!("on: {}", node.value)));
        }
        let items: Vec<String> = inner
            .split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        if items.is_empty() {
            return Err(outside(node.line, &format!("on: {}", node.value)));
        }
        return Ok(items);
    }
    if raw.is_empty() || raw.contains(['[', ']', ',']) {
        return Err(outside(node.line, &format!("on: {}", node.value)));
    }
    Ok(vec![raw.to_owned()])
}

fn permissions(node: Option<&Node>) -> Result<Option<BTreeMap<String, String>>, Failure> {
    let Some(node) = node else {
        return Ok(None);
    };
    if node.children.is_empty() {
        return Err(outside(node.line, &format!("permissions: {}", node.value)));
    }
    node.children
        .iter()
        .map(|grant| Ok((grant.key.clone(), flat_scalar(grant)?)))
        .collect::<Result<_, _>>()
        .map(Some)
}

fn step(item: &Node) -> Result<Step, Failure> {
    if item.key != "-" || item.children.is_empty() {
        return Err((item.line, "a step must be a `- ` mapping".to_owned()));
    }
    let with = match child(&item.children, "with") {
        Some(with) if with.children.is_empty() => {
            return Err((with.line, "`with` must be a mapping".to_owned()));
        },
        Some(with) => with
            .children
            .iter()
            .map(|c| Ok((c.line, c.key.clone(), flat_scalar(c)?)))
            .collect::<Result<_, Failure>>()?,
        None => Vec::new(),
    };
    Ok(Step {
        uses: child(&item.children, "uses").map(flat_scalar).transpose()?,
        with,
    })
}

fn job(node: &Node) -> Result<Job, Failure> {
    let steps = child(&node.children, "steps")
        .map(|s| s.children.iter().map(step).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();
    Ok(Job {
        line: node.line,
        permissions: permissions(child(&node.children, "permissions"))?,
        steps,
    })
}

pub fn workflow(
    nodes: &[Node],
    line: usize,
) -> Result<Workflow, Failure> {
    let jobs = child(nodes, "jobs")
        .map(|j| j.children.iter().map(job).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();
    Ok(Workflow {
        line,
        triggers: child(nodes, "on")
            .map(triggers)
            .transpose()?
            .unwrap_or_default(),
        permissions: permissions(child(nodes, "permissions"))?,
        jobs,
    })
}

pub fn workflows(page: &Page) -> Result<Vec<Workflow>, String> {
    page.blocks()
        .into_iter()
        .filter(|b| matches!(b.info.split_whitespace().next(), Some("yaml" | "yml")))
        .map(|b| {
            parse_yaml(&b.body, b.open_line + 1)
                .and_then(|nodes| workflow(&nodes, b.open_line))
                .map_err(|(line, message)| format!("{}: {message}", page.at(line)))
        })
        .collect()
}

fn names_action(uses: &str) -> bool {
    uses.starts_with("minikin/cargo-crap/actions/crapts")
}

fn uses(
    job: &Job,
    pred: impl Fn(&str) -> bool,
) -> Option<usize> {
    job.steps
        .iter()
        .position(|s| s.uses.as_deref().is_some_and(&pred))
}

pub fn check_with_keys(
    page: &Page,
    action: &ActionYml,
) -> Result<(), String> {
    for workflow in workflows(page)? {
        for step in workflow.jobs.iter().flat_map(|j| &j.steps) {
            if !step.uses.as_deref().is_some_and(names_action) {
                continue;
            }
            for (line, key, _) in &step.with {
                if !action.inputs.contains_key(key) {
                    return Err(format!(
                        "{}: {key} is not an input of the Action",
                        page.at(*line)
                    ));
                }
            }
        }
    }
    Ok(())
}

fn node_major(version: &str) -> Option<u32> {
    let digits: String = version.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The Action's scripts refuse a Node older than [`MIN_NODE`], so a job
/// that runs it must set one up first.
fn node_ready(
    job: &Job,
    action_at: usize,
) -> Result<(), String> {
    let setup = job.steps[..action_at]
        .iter()
        .rev()
        .find(|s| {
            s.uses
                .as_deref()
                .is_some_and(|u| u.starts_with("actions/setup-node@"))
        })
        .ok_or_else(|| "it does not set up Node before the crapts step".to_owned())?;
    let version = setup
        .with
        .iter()
        .find(|(_, k, _)| k == "node-version")
        .map_or_default(|(_, _, v)| v.as_str());
    match node_major(version) {
        Some(major) if major >= MIN_NODE => Ok(()),
        _ => Err(format!(
            "it sets up node-version {version:?}, the Action needs {MIN_NODE} or later"
        )),
    }
}

fn complete_job(
    workflow: &Workflow,
    job: &Job,
) -> Result<(), String> {
    let action_at = uses(job, names_action).ok_or_else(|| "it has no crapts step".to_owned())?;
    let granted = job.permissions.as_ref().or(workflow.permissions.as_ref());
    for (scope, level) in [("contents", "read"), ("pull-requests", "write")] {
        if granted.and_then(|p| p.get(scope)).map(String::as_str) != Some(level) {
            return Err(format!("it does not grant {scope}: {level}"));
        }
    }
    let checkout = uses(job, |u| u.starts_with("actions/checkout@"));
    if checkout.is_none_or(|at| at >= action_at) {
        return Err("it has no actions/checkout step before the crapts step".to_owned());
    }
    node_ready(job, action_at)?;
    let fed = job.steps[action_at]
        .with
        .iter()
        .any(|(_, k, v)| (k == "coverage" || k == "coverage-command") && !v.is_empty());
    if fed {
        Ok(())
    } else {
        Err("its crapts step sets neither coverage nor coverage-command".to_owned())
    }
}

pub fn check_complete_workflow(page: &Page) -> Result<(), String> {
    let mut reasons = Vec::new();
    for workflow in workflows(page)? {
        if !workflow.triggers.iter().any(|t| t == "pull_request") {
            reasons.push(format!(
                "{}: it does not trigger on pull_request",
                page.at(workflow.line)
            ));
            continue;
        }
        for job in &workflow.jobs {
            match complete_job(&workflow, job) {
                Ok(()) => return Ok(()),
                Err(reason) => reasons.push(format!("{}: {reason}", page.at(job.line))),
            }
        }
    }
    Err(format!(
        "{}: no complete pull request workflow:\n{}",
        page.path,
        reasons.join("\n")
    ))
}

pub fn check_node_versions(page: &Page) -> Result<usize, String> {
    let mut jobs = 0;
    for workflow in workflows(page)? {
        for job in &workflow.jobs {
            let Some(action_at) = uses(job, names_action) else {
                continue;
            };
            node_ready(job, action_at).map_err(|e| format!("{}: {e}", page.at(job.line)))?;
            jobs += 1;
        }
    }
    Ok(jobs)
}

pub fn check_uses_lines(page: &Page) -> Result<usize, String> {
    let mut named = 0;
    for line in page.lines() {
        let text = line.text.trim().trim_start_matches("- ").trim();
        let Some(uses) = text.strip_prefix("uses:").map(unquote_yaml) else {
            continue;
        };
        if !uses.starts_with("minikin/cargo-crap") {
            continue;
        }
        if uses != ACTION_USES {
            return Err(format!(
                "{}: `uses: {uses}` is not the crapts Action, `uses: {ACTION_USES}`",
                page.at(line.number)
            ));
        }
        named += 1;
    }
    Ok(named)
}
