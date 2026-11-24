mod common;

#[cfg(feature = "triage")]
#[expect(
    dead_code,
    reason = "both CLI acceptance suites use the same recording server"
)]
#[path = "../../../tests/support/api_stub.rs"]
mod api_stub;

#[cfg(feature = "triage")]
use serde_json::{Value, json};
#[cfg(feature = "triage")]
use std::path::Path;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};

struct Project {
    dir: tempfile::TempDir,
    source: PathBuf,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("project");
        let home = dir.path().join("home");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&home).unwrap();
        let project = Self { dir, source };
        project.config("");
        project
    }

    fn config(
        &self,
        settings: &str,
    ) {
        self.write(".crapts.toml", &format!("[duplicates]\nmin-nodes=0\nmin-similarity=1.0\n[duplicates.triage]\nenabled=true\n{settings}"));
    }

    fn write(
        &self,
        path: &str,
        contents: &str,
    ) {
        let path = self.source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn pair(
        &self,
        id: &str,
        body: &str,
    ) {
        for side in ["a", "b"] {
            self.write(
                &format!("{id}/{side}.ts"),
                &format!("function {id}_{side}(x){{{body}}}"),
            );
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn command(&self) -> Command {
        let mut cmd = common::binary(&self.source);
        cmd.env("HOME", self.home())
            .env("LOCALAPPDATA", self.home())
            .env("XDG_CACHE_HOME", self.home())
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env("TYPESAFE_BASE_URL", "http://127.0.0.1:1")
            .env("OPENAI_BASE_URL", "http://127.0.0.1:1/v1");
        cmd
    }

    fn run(
        &self,
        args: &[&str],
    ) -> Output {
        self.command()
            .args(["--duplicates", "--format", "json"])
            .args(args)
            .output()
            .unwrap()
    }
}

#[test]
fn s34_a_missing_client_or_selected_credential_preserves_the_score_gate() {
    let p = Project::new();
    p.pair("group", "if(x)return 1;return 0");
    p.write(
        "cov.info",
        "SF:group/a.ts\nDA:1,0\nend_of_record\nSF:group/b.ts\nDA:1,0\nend_of_record\n",
    );
    let args = [
        "--duplicates",
        "--coverage",
        "cov.info",
        "--format",
        "json",
        "--threshold",
        "1",
        "--fail-above",
    ];
    let out = p
        .command()
        .env("OPENAI_API_KEY", "fixture-other-provider")
        .args(args)
        .output()
        .unwrap();
    common::check(&out, 1);
    let doc = common::validated_report(&out.stdout);
    assert_eq!(doc["duplicate_groups"].as_array().unwrap().len(), 1);
    assert!(doc["duplicate_groups"][0].get("triage").is_none());
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(err.lines().count(), 1, "{err}");
    if cfg!(feature = "triage") {
        assert!(err.contains("TYPESAFE_API_KEY"), "{err}");
    } else {
        for word in ["@minikin/crapts", "cargo install crapts --features triage"] {
            assert!(err.contains(word), "{err}");
        }
    }
    fs::remove_file(p.source.join("group/b.ts")).unwrap();
    let empty = p.run(&[]);
    common::check(&empty, 0);
    let empty = String::from_utf8(empty.stderr).unwrap();
    assert_eq!(
        empty.contains("cargo install crapts"),
        !cfg!(feature = "triage"),
        "{empty}"
    );
    assert!(!empty.contains("TYPESAFE_API_KEY"));
}

#[cfg(feature = "triage")]
mod enabled {
    use super::*;
    use api_stub::{ApiStub, Reply};

    fn answer(openai: bool) -> Reply {
        let body = if openai {
            json!({"answers":[
                {"name":"duplication_kind","type":"choice","choice":"same_logic","confidence":0.50},
                {"name":"worth_extracting","type":"score","score":3.0,"confidence":1.0},
                {"name":"divergence_risk","type":"predicate","probability":0.66}]})
        } else {
            json!({"answers":{
                "duplication_kind":{"type":"choice","choice":"same_logic","confidence":0.50},
                "worth_extracting":{"type":"score","score":3.0,"confidence":1.0},
                "divergence_risk":{"type":"noul","noul":0.66}}})
        };
        Reply::json(&body.to_string())
    }

    fn ask(
        p: &Project,
        stub: &ApiStub,
        cwd: &Path,
        args: &[&str],
    ) -> Output {
        p.command()
            .current_dir(cwd)
            .env("TYPESAFE_API_KEY", "local-fixture")
            .env("OPENAI_API_KEY", "local-fixture")
            .env("TYPESAFE_BASE_URL", stub.base_url())
            .env("OPENAI_BASE_URL", format!("{}/v1", stub.base_url()))
            .args(["--duplicates", "--format", "json"])
            .args(args)
            .output()
            .unwrap()
    }

    fn doc(out: &Output) -> Value {
        common::check(out, 0);
        let raw: Value = serde_json::from_slice(&out.stdout).unwrap();
        if raw.get("regression_gate").is_some() {
            common::validated_delta(&out.stdout)
        } else {
            common::validated_report(&out.stdout)
        }
    }

    fn state(body: &Value) -> Value {
        body.get("state")
            .cloned()
            .unwrap_or_else(|| serde_json::from_str(body["input"].as_str().unwrap()).unwrap())
    }

    #[test]
    fn s27_s29_both_providers_ask_about_the_strongest_pair_and_reapply_the_floor_on_hits() {
        for openai in [false, true] {
            let p = Project::new();
            p.pair("f", "if (x) return x+1; return x*2");
            p.write("f/c.ts", "function third(y){if (y) return y+3; return y/4}");
            let cfg = if openai { "provider='openai'" } else { "" };
            p.config(cfg);
            let stub = ApiStub::scripted(vec![answer(openai)]);
            let flags = ["--dup-threshold", "0.1"];
            let raw = doc(&ask(&p, &stub, &p.source, &flags));
            assert_eq!(raw["duplicate_groups"].as_array().unwrap().len(), 1);
            assert_eq!(raw["duplicates"].as_array().unwrap().len(), 3);
            let calls = stub.requests();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].method, "POST");
            assert_eq!(
                calls[0].path,
                if openai {
                    "/v1/decisions"
                } else {
                    "/v1/systemone"
                }
            );
            let body = calls[0].json();
            assert_eq!(
                body["model"],
                if openai { "gpt-6-luna" } else { "jev-latest" }
            );
            let words = body["questions"].to_string();
            for word in [
                "duplication_kind",
                "worth_extracting",
                "divergence_risk",
                "JavaScript",
                "TypeScript",
            ] {
                assert!(words.contains(word), "{words}");
            }
            let state = state(&body);
            assert_eq!(state["function_a"]["name"], "f_a");
            assert_eq!(state["function_b"]["name"], "f_b");
            assert_eq!(state["structural_similarity"], 1.0);
            let v = &raw["duplicate_groups"][0]["triage"];
            assert_eq!(
                v,
                &json!({"kind":"same-logic","confidence":0.5,"worth_extracting_score":3.0,"worth_extracting":"should-be-one","divergence_risk":0.66})
            );
            assert!(
                raw["duplicates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|pair| pair.get("triage") == Some(v))
            );
            let gfm = ask(
                &p,
                &stub,
                &p.source,
                &["--dup-threshold", "0.1", "--format", "markdown"],
            );
            let gfm = String::from_utf8(gfm.stdout).unwrap();
            assert!(
                gfm.contains("**same-logic · should-be-one**")
                    && gfm.contains("⚠️ 0.66")
                    && gfm.contains("Triaged by"),
                "{gfm}"
            );
            p.config(&format!("{cfg}\nconfidence-floor=0.51"));
            let mut again = doc(&ask(&p, &stub, &p.source, &flags));
            assert_eq!(
                again["duplicate_groups"][0]["triage"],
                json!({"kind":"uncertain","confidence":0.5})
            );
            assert_eq!(stub.request_count(), 1);
            again["duplicate_groups"][0]["triage"] = v.clone();
            for pair in again["duplicates"].as_array_mut().unwrap() {
                pair["triage"] = v.clone();
            }
            assert_eq!(again, raw);
            p.config(&format!("{cfg}\nmodel='chosen-model'"));
            doc(&ask(&p, &stub, &p.source, &flags));
            assert_eq!(stub.requests()[1].json()["model"], "chosen-model");
        }
    }

    #[test]
    fn s28_s53_excerpts_select_exact_bytes_and_only_submitted_edits_cost_a_request() {
        let p = Project::new();
        p.write("a.ts", "hook(x=> x+1); leak(y=> -y);");
        p.write("b.ts", "const other = v=> v+2;");
        let stub = ApiStub::scripted(vec![answer(false)]);
        doc(&ask(&p, &stub, &p.source, &[]));
        let body = state(&stub.requests()[0].json());
        assert_eq!(body["function_a"]["source"], "x=> x+1");
        assert_eq!(body["function_b"]["source"], "v=> v+2");
        p.write("a.ts", "const unrelated = 42; hook(x=> x+1); leak(y=> -y);");
        doc(&ask(&p, &stub, &p.source, &[]));
        assert_eq!(stub.request_count(), 1);
        for (side, method) in [("a", "one"), ("b", "two")] {
            p.write(&format!("{side}.ts"),&format!("class C {{\r\n  field = init(42);\r\n  {method}() {{ {} }}\r\n  constructor() {{ this.go(); }}\r\n}}",if side=="a"{"return 3"}else{"throw 4"}));
        }
        doc(&ask(&p, &stub, &p.source, &[]));
        let body = state(&stub.requests()[1].json());
        for side in ["function_a", "function_b"] {
            let text = body[side]["source"].as_str().unwrap();
            assert!(
                text.contains("init(42)") && text.contains("constructor()"),
                "{body}"
            );
            assert!(
                !text.contains("one()") && !text.contains("two()") && !text.contains('\r'),
                "{body}"
            );
            let where_ = body[side]["location"].as_str().unwrap();
            assert!(where_.contains("2-2") && where_.contains("4-4"), "{body}");
        }
        for side in ["a", "b"] {
            p.write(
                &format!("{side}.ts"),
                &format!(
                    "function wrap_{side}(items) {{\r\n return items.map(x=>x+1 /*inside*/);\r\n}}"
                ),
            );
        }
        doc(&ask(&p, &stub, &p.source, &[]));
        let before = stub.request_count();
        p.write(
            "a.ts",
            "function wrap_a(items) {\n return items.map(x=>x+9 /*inside*/);\n}",
        );
        let newer = doc(&ask(&p, &stub, &p.source, &[]));
        assert!(newer["duplicate_groups"].as_array().unwrap().len() >= 2);
        let calls = stub.requests();
        assert!(calls[before..].iter().any(|r| {
            let body = state(&r.json());
            body["function_a"]["name"] == "wrap_a"
                && body["function_a"]["source"]
                    .as_str()
                    .unwrap()
                    .contains("x+9 /*inside*/")
        }));
    }

    #[test]
    fn s30_s31_cache_hits_precede_budget_and_credentials() {
        let p = Project::new();
        let stub = ApiStub::scripted(vec![answer(false)]);
        let all = [
            ("a", "return x"),
            ("b", "return !x"),
            ("c", "return x()"),
            ("d", "return [x]"),
        ];
        for (id, body) in all {
            p.pair(id, body);
        }
        doc(&ask(&p, &stub, &p.source, &["b", "d"]));
        assert_eq!(stub.request_count(), 2);
        p.config("max-requests=0");
        let out = p.run(&[]);
        let zero = doc(&out);
        assert_eq!(zero["untriaged_groups"], 2);
        for (i, g) in zero["duplicate_groups"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(g.get("triage").is_some(), i % 2 == 1);
        }
        let err = String::from_utf8(out.stderr).unwrap();
        assert!(!err.contains("TYPESAFE_API_KEY"), "{err}");
        let cold = doc(&p.run(&["a", "c"]));
        assert_eq!(cold["untriaged_groups"], 2);
        let gfm = p.run(&["a", "c", "--format", "markdown"]);
        let gfm = String::from_utf8(gfm.stdout).unwrap();
        assert!(
            gfm.contains("budget ran out") && !gfm.contains("Triaged by"),
            "{gfm}"
        );
        p.pair("Aold", "throw x");
        let empty_record = doc(&ask(&p, &stub, &p.source, &["Aold"]));
        p.write("baseline.json", &empty_record.to_string());
        p.config("max-requests=1");
        let out = ask(&p, &stub, &p.source, &["--baseline", "baseline.json"]);
        let delta = doc(&out);
        let groups = delta["duplicate_groups"].as_array().unwrap();
        assert_eq!(groups.len(), 5);
        assert_eq!(groups[0]["status"], "unchanged");
        assert!(groups[0].get("triage").is_none());
        assert_eq!(delta["untriaged_groups"], 1);
        for (i, g) in groups.iter().enumerate().skip(1) {
            assert_eq!(g.get("triage").is_some(), i != 3);
        }
        assert_eq!(stub.request_count(), 3);
        assert_eq!(
            state(&stub.requests()[2].json())["function_a"]["name"],
            "a_a"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert_eq!(
            stderr
                .lines()
                .filter(|line| line.contains("max-requests") || line.contains("triage budget"))
                .count(),
            1,
            "{stderr}"
        );
        for fmt in ["text", "markdown"] {
            p.config("max-requests=0");
            let out = p.run(&["--baseline", "baseline.json", "--format", fmt]);
            let text = String::from_utf8(out.stdout).unwrap();
            assert!(text.contains("budget ran out"), "{text}");
        }
    }

    #[test]
    fn s18_missing_empty_and_unchanged_history_control_eligibility() {
        let p = Project::new();
        let stub = ApiStub::scripted(vec![answer(false)]);
        for (id, body) in [("b", "return !x"), ("c", "return x()"), ("d", "return [x]")] {
            p.pair(id, body);
        }
        let two = doc(&ask(&p, &stub, &p.source, &["b", "d"]));
        p.write("baseline.json", &two.to_string());
        let before = stub.request_count();
        let out = ask(
            &p,
            &stub,
            &p.source,
            &[
                "b",
                "d",
                "--baseline",
                "baseline.json",
                "--format",
                "pr-comment",
            ],
        );
        common::check(&out, 0);
        assert!(
            !String::from_utf8(out.stdout)
                .unwrap()
                .contains("duplicate candidate")
        );
        assert_eq!(stub.request_count(), before);
        let mut old = two.clone();
        for name in ["duplicates", "duplicate_groups", "duplicate_context"] {
            old.as_object_mut().unwrap().remove(name);
        }
        p.write("baseline.json", &old.to_string());
        let out = ask(&p, &stub, &p.source, &["c", "--baseline", "baseline.json"]);
        let legacy = doc(&out);
        let g = &legacy["duplicate_groups"][0];
        assert!(g.get("status").is_none() && g.get("triage").is_some());
        assert_eq!(
            String::from_utf8(out.stderr)
                .unwrap()
                .lines()
                .filter(|s| s.contains("no duplicate record") && s.contains("--duplicates"))
                .count(),
            1
        );
        for field in ["duplicates", "duplicate_groups", "duplicate_context"] {
            old[field] = two[field].clone();
        }
        old["duplicates"] = json!([]);
        old["duplicate_groups"] = json!([]);
        old["duplicate_context"]["families"] = json!([]);
        p.write("baseline.json", &old.to_string());
        let out = ask(&p, &stub, &p.source, &["c", "--baseline", "baseline.json"]);
        assert_eq!(doc(&out)["duplicate_groups"][0]["status"], "new");
        assert!(
            !String::from_utf8(out.stderr)
                .unwrap()
                .contains("no duplicate record")
        );
    }

    #[test]
    fn s35_s51_failed_batches_drop_every_assessment_and_retry_only_transient_responses() {
        for (sequence, expected, success) in [
            (vec![Reply::status(429), answer(false)], 2, true),
            (vec![Reply::status(503)], 3, false),
            (vec![Reply::status(401)], 1, false),
            (vec![Reply::json("{}")], 1, false),
        ] {
            let p = Project::new();
            p.pair("old", "return x");
            let warm = ApiStub::scripted(vec![answer(false)]);
            doc(&ask(&p, &warm, &p.source, &[]));
            p.pair("new", "throw x");
            p.config("max-requests=1");
            let stub = ApiStub::scripted(sequence);
            let out = ask(&p, &stub, &p.source, &[]);
            let report = doc(&out);
            assert_eq!(stub.request_count(), expected);
            for g in report["duplicate_groups"].as_array().unwrap() {
                assert_eq!(g.get("triage").is_some(), success);
            }
            if !success {
                assert_eq!(
                    String::from_utf8(out.stderr)
                        .unwrap()
                        .lines()
                        .filter(|l| l.contains("triage skipped"))
                        .count(),
                    1
                );
                doc(&ask(&p, &warm, &p.source, &[]));
                assert_eq!(warm.request_count(), 2);
            }
        }
    }

    #[test]
    fn s51_a_fresh_verdict_finishes_before_a_later_refusal_and_survives_it() {
        let p = Project::new();
        p.config("provider='openai'");
        p.pair("first", "return x");
        p.pair("second", "throw x");
        let mixed = ApiStub::respond_with(|request| {
            if request.body.contains("second_a") {
                Reply::delayed(
                    std::time::Duration::from_millis(300),
                    Reply::json(r#"{"refusal":"unable to judge"}"#),
                )
            } else {
                answer(true)
            }
        });
        let out = ask(&p, &mixed, &p.source, &[]);
        let failed = doc(&out);
        assert_eq!(mixed.request_count(), 2);
        assert!(
            failed["duplicate_groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|g| g.get("triage").is_none())
        );
        assert_eq!(
            String::from_utf8(out.stderr)
                .unwrap()
                .lines()
                .filter(|s| s.contains("triage skipped"))
                .count(),
            1
        );
        let retry = ApiStub::scripted(vec![answer(true)]);
        let recovered = doc(&ask(&p, &retry, &p.source, &[]));
        assert!(
            recovered["duplicate_groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|g| g.get("triage").is_some())
        );
        assert_eq!(retry.request_count(), 1);
        assert!(retry.requests()[0].body.contains("second_a"));
    }

    #[test]
    fn s33_a_cache_root_in_the_worktree_remains_empty_while_paid_triage_works() {
        let p = Project::new();
        p.pair("sub", "return x");
        git(&p, &["init", "-q"]);
        let inner = p.source.join("cache-root");
        let stub = ApiStub::scripted(vec![answer(false)]);
        let out = p
            .command()
            .current_dir(p.source.join("sub"))
            .env("HOME", &inner)
            .env("LOCALAPPDATA", &inner)
            .env("XDG_CACHE_HOME", &inner)
            .env("TYPESAFE_BASE_URL", stub.base_url())
            .env("TYPESAFE_API_KEY", "local-fixture")
            .args(["--duplicates", "--format", "json"])
            .output()
            .unwrap();
        assert!(doc(&out)["duplicate_groups"][0].get("triage").is_some());
        assert!(!inner.exists());
        assert_eq!(stub.request_count(), 1);
        let err = String::from_utf8(out.stderr).unwrap();
        assert_eq!(
            err.lines()
                .filter(|l| l.contains("cache") && l.contains("worktree"))
                .count(),
            1,
            "{err}"
        );
    }

    fn cache_dir(base: &Path) -> PathBuf {
        let root = if cfg!(target_os = "macos") {
            base.join("Library/Caches")
        } else {
            base.to_path_buf()
        };
        root.join("crapts/triage")
    }

    fn git(
        p: &Project,
        args: &[&str],
    ) -> String {
        let out = Command::new("git")
            .current_dir(&p.source)
            .env("HOME", p.home())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_CONFIG_GLOBAL")
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .args(args)
            .output()
            .unwrap();
        common::check(&out, 0);
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn s32_s33_the_external_cache_survives_relocation_and_repairs_corrupt_entries() {
        let p = Project::new();
        p.pair("src", "return x");
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["add", "."]);
        git(&p, &["commit", "-qm", "fixture"]);
        let stub = ApiStub::scripted(vec![answer(false)]);
        let report = doc(&ask(&p, &stub, &p.source, &["src"]));
        let dir = cache_dir(&p.home());
        let files: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|item| item.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1);
        let file = &files[0];
        let name = file.file_stem().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 64);
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        );
        let entry: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
        assert_eq!(entry.as_object().unwrap().len(), 4);
        for field in ["kind", "confidence", "worth_extracting", "divergence_risk"] {
            assert!(entry.get(field).is_some());
        }
        assert_eq!(git(&p, &["status", "--porcelain"]), "");
        let changed = doc(&ask(&p, &stub, &p.source, &["--base", "main"]));
        assert_eq!(changed["changed"]["head"]["dirty"], false);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let cached = doc(&ask(&p, &stub, &p.source.join("src"), &[]));
                    assert_eq!(
                        cached["duplicate_groups"][0]["triage"],
                        report["duplicate_groups"][0]["triage"]
                    );
                });
            }
        });
        let mtime = fs::metadata(file).unwrap().modified().unwrap();
        let relocated = p.dir.path().join("other");
        fs::create_dir(&relocated).unwrap();
        fs::copy(
            p.source.join(".crapts.toml"),
            relocated.join(".crapts.toml"),
        )
        .unwrap();
        for f in ["a", "b"] {
            fs::copy(
                p.source.join(format!("src/{f}.ts")),
                relocated.join(format!("{f}.ts")),
            )
            .unwrap();
        }
        for cwd in [&p.source.join("src"), &relocated] {
            let cached = doc(&ask(&p, &stub, cwd, &[]));
            assert_eq!(
                cached["duplicate_groups"][0]["triage"],
                report["duplicate_groups"][0]["triage"]
            );
        }
        assert_eq!(mtime, fs::metadata(file).unwrap().modified().unwrap());
        assert_eq!(stub.request_count(), 1);
        fs::write(file, "unusable verdict").unwrap();
        doc(&ask(&p, &stub, &p.source, &[]));
        doc(&ask(&p, &stub, &p.source, &[]));
        assert_eq!(stub.request_count(), 2);
        let original = fs::read_to_string(p.source.join("src/b.ts")).unwrap();
        p.write(
            "src/b.ts",
            &original.replace("return x", "return x /*new*/"),
        );
        doc(&ask(&p, &stub, &p.source, &[]));
        assert_eq!(stub.request_count(), 3);
        p.config("provider='openai'");
        let openai = ApiStub::scripted(vec![answer(true)]);
        doc(&ask(&p, &openai, &p.source, &[]));
        assert_eq!(openai.request_count(), 1);
    }

    #[test]
    fn s33_an_unwritable_cache_costs_one_warning_and_keeps_the_verdicts() {
        let p = Project::new();
        p.pair("a", "return x");
        p.pair("b", "throw x");
        let cache = cache_dir(&p.home());
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(cache, "file in place of a verdict directory").unwrap();
        let stub = ApiStub::scripted(vec![answer(false)]);
        let out = ask(&p, &stub, &p.source, &[]);
        let raw = doc(&out);
        assert!(
            raw["duplicate_groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|g| g.get("triage").is_some())
        );
        let err = String::from_utf8(out.stderr).unwrap();
        assert_eq!(
            err.lines().filter(|s| s.contains("cache")).count(),
            1,
            "{err}"
        );
    }
}
