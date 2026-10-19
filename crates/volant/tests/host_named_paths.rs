// SPDX-License-Identifier: GPL-3.0-or-later
//! `[volant] host_named_paths`: what a `template`, `copy`, `script` or `unarchive` task does with
//! a controller `src` that a managed host named, in each of the three modes.
//!
//! One playbook per mode holds every pairing of a plugin and a way of naming the source. The
//! rows are the tasks of a role, each with `ignore_errors`, and the outcome of each is read from
//! the output. The plugins run on the controller's ansible-core, so the tests skip when the
//! recorded ansible-core is not there, as the scenarios do.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rstest::rstest;

#[path = "common/process.rs"]
mod process;

#[path = "common/python.rs"]
mod python;

const DEADLINE: Duration = Duration::from_secs(240);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plugin {
    Template,
    Copy,
    Script,
    Unarchive,
}

impl Plugin {
    const ALL: [Plugin; 4] = [
        Plugin::Template,
        Plugin::Copy,
        Plugin::Script,
        Plugin::Unarchive,
    ];

    fn name(self) -> &'static str {
        match self {
            Plugin::Template => "template",
            Plugin::Copy => "copy",
            Plugin::Script => "script",
            Plugin::Unarchive => "unarchive",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Plugin::Template => "j2",
            Plugin::Copy => "txt",
            Plugin::Script => "sh",
            Plugin::Unarchive => "tar",
        }
    }

    /// The argument the refusal names: a script's file is the first word of its free-form line.
    fn arg(self) -> &'static str {
        if self == Plugin::Script {
            "_raw_params"
        } else {
            "src"
        }
    }
}

/// How a task's `src` comes to be named by a managed host, or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    /// Written in the playbook: no host named it.
    Literal,
    /// `{{ ansible_system }}.<ext>`, a gathered fact.
    Fact,
    /// The output of a `command`, a plain name under the role's directories.
    Registered,
    /// `with_first_found` over a fact-named file and a fallback: the item is an absolute path.
    FirstFound,
    /// The output of a `command`: an absolute path outside the playbook directory.
    Absolute,
    /// The output of a `command`: an absolute path inside the role.
    AbsoluteInside,
    /// The output of a `command`: a relative path with `..` components, which leaves the role.
    DotDot,
    /// The output of a `command`: a link inside the role that points outside it.
    Symlink,
    /// The output of a `command`: a path two directories down.
    Nested,
}

impl Case {
    const ALL: [Case; 9] = [
        Case::Literal,
        Case::Fact,
        Case::Registered,
        Case::FirstFound,
        Case::Absolute,
        Case::AbsoluteInside,
        Case::DotDot,
        Case::Symlink,
        Case::Nested,
    ];

    fn name(self) -> &'static str {
        match self {
            Case::Literal => "literal",
            Case::Fact => "fact",
            Case::Registered => "registered",
            Case::FirstFound => "first_found",
            Case::Absolute => "absolute",
            Case::AbsoluteInside => "absolute_inside",
            Case::DotDot => "dotdot",
            Case::Symlink => "symlink",
            Case::Nested => "nested",
        }
    }
}

/// What a task came to.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Changed,
    Refused(String),
    Other(String),
}

struct Fixture {
    root: PathBuf,
    play: PathBuf,
    outside: PathBuf,
    out: PathBuf,
    system: &'static str,
}

impl Fixture {
    fn role_files(&self) -> PathBuf {
        self.play.join("roles/r/files")
    }

    /// The text the task names the source with, and the `command` that produces it when a host
    /// is to name it.
    fn src(&self, plugin: Plugin, case: Case) -> (String, Option<String>) {
        let ext = plugin.ext();
        match case {
            Case::Literal => (format!("lit.{ext}"), None),
            Case::Fact => (format!("{{{{ ansible_system }}}}.{ext}"), None),
            Case::FirstFound => ("{{ item }}".into(), None),
            Case::Registered => (String::new(), Some(format!("reg.{ext}"))),
            Case::Absolute => (
                String::new(),
                Some(format!("{}/abs.{ext}", self.outside.display())),
            ),
            Case::AbsoluteInside => (
                String::new(),
                Some(format!("{}/reg.{ext}", self.role_files().display())),
            ),
            Case::DotDot => (String::new(), Some(format!("../../../../outside/up.{ext}"))),
            Case::Symlink => (String::new(), Some(format!("link.{ext}"))),
            Case::Nested => (String::new(), Some(format!("sub/dir/nested.{ext}"))),
        }
    }

    /// The file the source ends up being, named as it labels its own content.
    fn label(&self, plugin: Plugin, case: Case) -> String {
        let ext = plugin.ext();
        match case {
            Case::Literal => format!("lit.{ext}"),
            Case::Fact | Case::FirstFound => format!("{}.{ext}", self.system),
            Case::Registered | Case::AbsoluteInside => format!("reg.{ext}"),
            Case::Absolute => format!("abs.{ext}"),
            Case::DotDot => format!("up.{ext}"),
            Case::Symlink => format!("target.{ext}"),
            Case::Nested => format!("nested.{ext}"),
        }
    }

    /// Where the task's result lands.
    fn dest(&self, plugin: Plugin, case: Case) -> PathBuf {
        let stem = format!("{}-{}", plugin.name(), case.name());
        match plugin {
            Plugin::Template | Plugin::Copy => self.out.join(format!("{stem}.txt")),
            Plugin::Script => self.out.join(format!("{stem}.none")),
            Plugin::Unarchive => self.out.join(&stem),
        }
    }

    /// The file a finished task leaves behind, with what it holds.
    fn product(&self, plugin: Plugin, case: Case) -> Option<(PathBuf, String)> {
        let label = self.label(plugin, case);
        match plugin {
            Plugin::Template => Some((self.dest(plugin, case), format!("{label} 2=2\n"))),
            Plugin::Copy => Some((self.dest(plugin, case), format!("{label}\n"))),
            Plugin::Unarchive => Some((
                self.dest(plugin, case).join("payload"),
                format!("{label}\n"),
            )),
            Plugin::Script => None,
        }
    }
}

/// A file's content, as the plugin reads it.
fn write_source(path: &Path, plugin: Plugin, label: &str) {
    std::fs::create_dir_all(path.parent().expect("a file has a directory")).unwrap();
    match plugin {
        Plugin::Template => std::fs::write(path, format!("{label} 2={{{{ 1 + 1 }}}}\n")).unwrap(),
        Plugin::Copy => std::fs::write(path, format!("{label}\n")).unwrap(),
        Plugin::Script => std::fs::write(path, format!("#!/bin/sh\necho {label}\n")).unwrap(),
        Plugin::Unarchive => {
            let scratch = tempfile::tempdir().unwrap();
            std::fs::write(scratch.path().join("payload"), format!("{label}\n")).unwrap();
            let made = std::process::Command::new("tar")
                .arg("-cf")
                .arg(path)
                .arg("-C")
                .arg(scratch.path())
                .arg("payload")
                .status()
                .expect("tar runs");
            assert!(made.success(), "tar made {}", path.display());
        }
    }
}

fn fixture(mode: &str) -> (tempfile::TempDir, Fixture) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let fx = Fixture {
        play: root.join("play"),
        outside: root.join("outside"),
        out: root.join("out"),
        system: if cfg!(target_os = "macos") {
            "Darwin"
        } else {
            "Linux"
        },
        root,
    };
    std::fs::create_dir_all(&fx.out).unwrap();
    std::fs::create_dir_all(&fx.outside).unwrap();
    for plugin in Plugin::ALL {
        let ext = plugin.ext();
        for folder in ["files", "templates"] {
            let base = fx.play.join("roles/r").join(folder);
            for name in [
                format!("{}.{ext}", fx.system),
                format!("reg.{ext}"),
                format!("default.{ext}"),
                format!("lit.{ext}"),
                format!("sub/dir/nested.{ext}"),
            ] {
                let label = Path::new(&name)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                write_source(&base.join(&name), plugin, &label);
            }
            std::os::unix::fs::symlink(
                fx.outside.join(format!("target.{ext}")),
                base.join(format!("link.{ext}")),
            )
            .unwrap();
        }
        for name in ["abs", "up", "target"] {
            write_source(
                &fx.outside.join(format!("{name}.{ext}")),
                plugin,
                &format!("{name}.{ext}"),
            );
        }
        if plugin == Plugin::Unarchive {
            for case in Case::ALL {
                std::fs::create_dir_all(fx.dest(plugin, case)).unwrap();
            }
        }
    }
    std::fs::write(fx.play.join("inv.ini"), "h1 ansible_connection=local\n").unwrap();
    std::fs::write(
        fx.play.join("site.yml"),
        "- hosts: h1\n  gather_facts: true\n  gather_subset: [min]\n  roles: [r]\n",
    )
    .unwrap();
    std::fs::write(
        fx.play.join("ansible.cfg"),
        format!("[volant]\nhost_named_paths = {mode}\n"),
    )
    .unwrap();
    let mut tasks = String::new();
    for plugin in Plugin::ALL {
        for case in Case::ALL {
            tasks.push_str(&task(&fx, plugin, case));
        }
    }
    std::fs::create_dir_all(fx.play.join("roles/r/tasks")).unwrap();
    std::fs::write(fx.play.join("roles/r/tasks/main.yml"), tasks).unwrap();
    (dir, fx)
}

/// The YAML of the tasks of one row: the `command` that makes a host name the source, when there
/// is one, and the task that reads it.
fn task(fx: &Fixture, plugin: Plugin, case: Case) -> String {
    let id = format!("{}/{}", plugin.name(), case.name());
    let (src, named) = fx.src(plugin, case);
    let mut out = String::new();
    let src = match named {
        Some(value) => {
            let var = format!("n_{}_{}", plugin.name(), case.name());
            write!(
                out,
                "- name: \"{id}/name\"\n  command: echo {value}\n  register: {var}\n  changed_when: false\n  check_mode: false\n"
            )
            .unwrap();
            format!("{{{{ {var}.stdout }}}}")
        }
        None => src,
    };
    let dest = fx.dest(plugin, case).display().to_string();
    let action = match plugin {
        Plugin::Template | Plugin::Copy => {
            format!("{}: {{src: \"{src}\", dest: \"{dest}\"}}", plugin.name())
        }
        Plugin::Script => format!("script: \"{src}\""),
        Plugin::Unarchive => format!("unarchive: {{src: \"{src}\", dest: \"{dest}\"}}"),
    };
    write!(out, "- name: \"{id}\"\n  {action}\n  ignore_errors: true\n").unwrap();
    if case == Case::FirstFound {
        let ext = plugin.ext();
        writeln!(
            out,
            "  with_first_found:\n    - \"{{{{ ansible_system }}}}.{ext}\"\n    - default.{ext}"
        )
        .unwrap();
    }
    out
}

/// A run of Volant on the playbook, with nothing of the caller's environment but what is named.
fn run(fx: &Fixture, python: &Path, flags: &[&str]) -> std::process::Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_volant"));
    command
        .args(["playbook", "-i", "inv.ini", "site.yml"])
        .args(flags)
        .current_dir(&fx.play);
    for (name, _) in std::env::vars() {
        if name.starts_with("ANSIBLE_") || name.starts_with("VOLANT_") {
            command.env_remove(name);
        }
    }
    command
        .env("NO_COLOR", "1")
        .env_remove("COLUMNS")
        .env("VOLANT_RUNTIME", "system")
        .env("VOLANT_PYTHON", python)
        .env("VOLANT_REMOTE_TMP", fx.root.join("tmp"))
        .env("ANSIBLE_CONFIG", fx.play.join("ansible.cfg"));
    process::run_within(&mut command, DEADLINE, None)
}

/// Each task of the role, with what its block of the output says.
fn outcomes(stdout: &str) -> BTreeMap<String, Outcome> {
    let refusal =
        regex::Regex::new(r#"the '[_a-z]+' of this task was named by a managed host[^"\n]*"#)
            .expect("the pattern compiles");
    let mut found = BTreeMap::new();
    for block in stdout.split("\nTASK [").skip(1) {
        let Some(name) = block.strip_prefix("r : ").and_then(|b| b.split(']').next()) else {
            continue;
        };
        if name.ends_with("/name") {
            continue;
        }
        let outcome = if let Some(said) = refusal.find(block) {
            Outcome::Refused(said.as_str().to_string())
        } else if block.contains("\nchanged: [h1]") {
            Outcome::Changed
        } else {
            Outcome::Other(block.to_string())
        };
        found.insert(name.to_string(), outcome);
    }
    found
}

/// What the mode says about a source named this way.
fn expected(mode: &str, fx: &Fixture, plugin: Plugin, case: Case) -> Outcome {
    let (_, named) = fx.src(plugin, case);
    let arg = plugin.arg();
    match (mode, case) {
        ("allow", _) | (_, Case::Literal) => Outcome::Changed,
        ("refuse", _) => Outcome::Refused(format!(
            "the '{arg}' of this task was named by a managed host, and a controller file a host chose is never sent"
        )),
        ("contained", Case::Absolute | Case::DotDot) => Outcome::Refused(format!(
            "the '{arg}' of this task was named by a managed host, and '{}' is not inside the directories of the playbook or the role (host_named_paths = contained)",
            named.expect("a host named it")
        )),
        ("contained", _) => Outcome::Changed,
        (other, _) => panic!("no such mode: {other}"),
    }
}

/// A source a managed host named is read or refused as the mode says, whatever the plugin. Under
/// `--check` the file-sending plugins run as ansible-core's own, in the Python helper, which
/// is a different road to the same answer: only the refusals are compared there.
#[rstest]
#[case::allow("allow", &[])]
#[case::contained("contained", &[])]
#[case::refuse("refuse", &[])]
#[case::contained_check("contained", &["--check"])]
#[case::refuse_check("refuse", &["--check"])]
fn a_source_a_host_named_follows_the_mode(#[case] mode: &str, #[case] flags: &[&str]) {
    let Some(python) = python::reference_python() else {
        return;
    };
    let (_dir, fx) = fixture(mode);
    let out = run(&fx, &python, flags);
    let checking = flags.contains(&"--check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let got = outcomes(&stdout);
    let mut wrong = Vec::new();
    for plugin in Plugin::ALL {
        for case in Case::ALL {
            let id = format!("{}/{}", plugin.name(), case.name());
            let want = expected(mode, &fx, plugin, case);
            let ran = got.get(&id);
            let same = match (checking, &want, ran) {
                (false, _, _) | (true, Outcome::Refused(_), _) => ran == Some(&want),
                (true, _, Some(Outcome::Refused(_)) | None) => false,
                (true, _, Some(_)) => true,
            };
            if !same {
                wrong.push(format!("{id}: wanted {want:?}, got {ran:?}"));
                continue;
            }
            let Some((file, content)) = fx.product(plugin, case).filter(|_| !checking) else {
                continue;
            };
            let found = std::fs::read_to_string(&file).ok();
            let wanted = (want == Outcome::Changed).then_some(content);
            if found != wanted {
                wrong.push(format!(
                    "{id}: {} holds {found:?}, wanted {wanted:?}",
                    file.display()
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "mode {mode}:\n{}\n--- stdout\n{stdout}\n--- stderr\n{stderr}",
        wrong.join("\n")
    );
}

/// A value that is none of the three stops the run before it starts, from the file and from the
/// environment, and says which key, which values and what was found.
#[rstest]
#[case::file(false)]
#[case::environment(true)]
fn a_mode_that_is_none_of_the_three_stops_the_run(#[case] from_env: bool) {
    let dir = tempfile::tempdir().unwrap();
    let play = dir.path().join("play.yml");
    std::fs::write(
        &play,
        "- hosts: localhost\n  gather_facts: false\n  tasks:\n    - debug: {msg: started}\n",
    )
    .unwrap();
    let config = dir.path().join("ansible.cfg");
    let (file_value, origin) = if from_env {
        ("allow", "env: VOLANT_HOST_NAMED_PATHS".to_string())
    } else {
        ("sometimes", config.display().to_string())
    };
    std::fs::write(
        &config,
        format!("[volant]\nhost_named_paths = {file_value}\n"),
    )
    .unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_volant"));
    command.arg("playbook").arg(&play).current_dir(dir.path());
    for (name, _) in std::env::vars() {
        if name.starts_with("ANSIBLE_") || name.starts_with("VOLANT_") {
            command.env_remove(name);
        }
    }
    command
        .env("NO_COLOR", "1")
        .env("VOLANT_RUNTIME", "system")
        .env("ANSIBLE_CONFIG", &config);
    if from_env {
        command.env("VOLANT_HOST_NAMED_PATHS", "sometimes");
    }
    let out = process::run_within(&mut command, DEADLINE, None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(5), "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "Config 'VOLANT_HOST_NAMED_PATHS' from '{origin}' has an invalid value: 'sometimes'; expected one of allow, contained, refuse"
        )),
        "{stderr}"
    );
    assert!(!stdout.contains("PLAY ["), "the run started:\n{stdout}");
}
