use crate::fixture::VERSION;
use crate::pages::{self, Page};
use std::collections::{BTreeMap, BTreeSet};

pub const ACTION_GUIDE: &str = "docs/crapts/guides/github-action.md";
pub const ACTION_USES: &str = "minikin/cargo-crap/actions/crapts@crapts-v1";

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
        if !line.starts_with(' ') && !line.trim().is_empty() {
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
    Ok(pages::table_rows(&section)
        .into_iter()
        .map(|(line, cells)| (line, pages::unquote(&cells[0]).to_owned(), cells))
        .collect())
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
        let stated = cells.get(1).map(|c| pages::unquote(c)).unwrap_or_default();
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

pub fn parse_yaml(
    body: &str,
    first_line: usize,
) -> Result<Vec<Node>, String> {
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
        Some(raw) => Err(format!("line {}: unexpected indentation", raw.line)),
    }
}

fn parse_block(
    raws: &mut [Raw],
    at: &mut usize,
    indent: usize,
) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    while let Some(raw) = raws.get(*at) {
        if raw.indent < indent {
            break;
        }
        if raw.indent > indent {
            return Err(format!("line {}: unexpected indentation", raw.line));
        }
        let line = raw.line;
        if let Some(rest) = raw.text.strip_prefix("- ").map(str::to_owned) {
            if !rest.contains(": ") && !rest.ends_with(':') {
                *at += 1;
                nodes.push(Node {
                    line,
                    key: "-".to_owned(),
                    value: unquote_yaml(&rest),
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
            .ok_or_else(|| format!("line {line}: `{}` is outside the workflow subset", raw.text))?;
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
            value: unquote_yaml(&value),
            children,
        });
    }
    Ok(nodes)
}

pub struct Step {
    pub uses: Option<String>,
    pub with: Vec<(usize, String, String)>,
}

pub struct Workflow {
    pub line: usize,
    pub triggers: Vec<String>,
    pub permissions: BTreeMap<String, String>,
    pub steps: Vec<Step>,
}

fn child<'a>(
    nodes: &'a [Node],
    key: &str,
) -> Option<&'a Node> {
    nodes.iter().find(|n| n.key == key)
}

fn triggers(node: &Node) -> Vec<String> {
    if node.children.is_empty() {
        node.value
            .trim_matches(['[', ']'])
            .split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        node.children.iter().map(|c| c.key.clone()).collect()
    }
}

pub fn workflow(
    nodes: &[Node],
    line: usize,
) -> Workflow {
    let mut permissions = BTreeMap::new();
    let mut steps = Vec::new();
    let mut scopes: Vec<&Node> = child(nodes, "permissions").into_iter().collect();
    for job in child(nodes, "jobs")
        .map(|j| j.children.as_slice())
        .unwrap_or_default()
    {
        scopes.extend(child(&job.children, "permissions"));
        for item in child(&job.children, "steps")
            .map(|s| s.children.as_slice())
            .unwrap_or_default()
        {
            steps.push(Step {
                uses: child(&item.children, "uses").map(|u| u.value.clone()),
                with: child(&item.children, "with")
                    .map(|w| {
                        w.children
                            .iter()
                            .map(|c| (c.line, c.key.clone(), c.value.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    for scope in scopes {
        for grant in &scope.children {
            permissions.insert(grant.key.clone(), grant.value.clone());
        }
    }
    Workflow {
        line,
        triggers: child(nodes, "on").map(triggers).unwrap_or_default(),
        permissions,
        steps,
    }
}

pub fn workflows(page: &Page) -> Result<Vec<Workflow>, String> {
    page.blocks()
        .into_iter()
        .filter(|b| matches!(b.info.as_str(), "yaml" | "yml"))
        .map(|b| {
            parse_yaml(&b.body, b.open_line + 1)
                .map(|nodes| workflow(&nodes, b.open_line))
                .map_err(|e| format!("{}: {e}", page.path))
        })
        .collect()
}

fn names_action(uses: &str) -> bool {
    uses.starts_with("minikin/cargo-crap/actions/crapts")
}

pub fn check_with_keys(
    page: &Page,
    action: &ActionYml,
) -> Result<(), String> {
    for workflow in workflows(page)? {
        for step in &workflow.steps {
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

fn complete(workflow: &Workflow) -> Result<(), String> {
    if !workflow.triggers.iter().any(|t| t == "pull_request") {
        return Err("it does not trigger on pull_request".to_owned());
    }
    for (scope, level) in [("contents", "read"), ("pull-requests", "write")] {
        if workflow.permissions.get(scope).map(String::as_str) != Some(level) {
            return Err(format!("it does not grant {scope}: {level}"));
        }
    }
    let checkout = workflow.steps.iter().any(|s| {
        s.uses
            .as_deref()
            .is_some_and(|u| u.starts_with("actions/checkout@"))
    });
    if !checkout {
        return Err("it has no actions/checkout step".to_owned());
    }
    let fed = workflow
        .steps
        .iter()
        .filter(|s| s.uses.as_deref().is_some_and(names_action))
        .any(|step| {
            step.with
                .iter()
                .any(|(_, k, v)| (k == "coverage" || k == "coverage-command") && !v.is_empty())
        });
    if fed {
        Ok(())
    } else {
        Err("its crapts step sets neither coverage nor coverage-command".to_owned())
    }
}

pub fn check_complete_workflow(page: &Page) -> Result<(), String> {
    let mut reasons = Vec::new();
    for workflow in workflows(page)? {
        match complete(&workflow) {
            Ok(()) => return Ok(()),
            Err(reason) => reasons.push(format!("{}: {reason}", page.at(workflow.line))),
        }
    }
    Err(format!(
        "{}: no complete pull request workflow:\n{}",
        page.path,
        reasons.join("\n")
    ))
}

pub fn check_uses_lines(page: &Page) -> Result<usize, String> {
    let mut named = 0;
    for line in page.lines() {
        let text = line.text.trim().trim_start_matches("- ").trim();
        let Some(uses) = text.strip_prefix("uses:").map(str::trim) else {
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
