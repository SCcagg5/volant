// SPDX-License-Identifier: GPL-3.0-or-later
//! A play's `hosts` written as a template: what the run and the listings print on stdout and
//! stderr. Every row was measured on ansible-core 2.19.12. The recorded scenarios
//! `play-hosts-templated*` pin the stdout of the same behaviour; the rows here pin the exit code
//! and the warnings and errors, which a scenario does not record.
#![cfg(unix)]
use std::process::Command;

use rstest::rstest;

#[path = "common/process.rs"]
mod process;

#[path = "common/python.rs"]
mod python;

const INVENTORY: &str = "[servers]\ns1\ns2\n\n[web]\nw1\n\n[all:vars]\nansible_connection=local\nansible_python_interpreter=/usr/bin/python3\n";

/// One play over `hosts` that prints the host it runs on.
fn play(hosts: &str) -> String {
    format!(
        "- hosts: {hosts}\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: \"ran {{{{ inventory_hostname }}}}\"\n"
    )
}

/// Runs volant over `playbook` and `INVENTORY` in a fresh directory: exit code, stdout, stderr.
fn run(name: &str, playbook: &str, args: &[&str]) -> (i32, String, String) {
    let dir = fresh(name, playbook);
    let out = run_in(&dir, args, None);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn fresh(name: &str, playbook: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("volant-play-hosts-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the directory is made");
    std::fs::write(dir.join("inventory.ini"), INVENTORY).expect("the inventory is written");
    std::fs::write(dir.join("site.yml"), playbook).expect("the playbook is written");
    dir
}

/// Runs volant in `dir`, with `python` as the controller's interpreter when there is one.
fn run_in(
    dir: &std::path::Path,
    args: &[&str],
    python: Option<&std::path::Path>,
) -> (i32, String, String) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_volant"));
    command
        .arg("playbook")
        .args(["-i", "inventory.ini"])
        .args(args)
        .arg("site.yml")
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .env("NO_COLOR", "1")
        .env_remove("VIRTUAL_ENV")
        .env_remove("VOLANT_PYTHON")
        .env("VOLANT_RUNTIME", "system");
    if let Some(python) = python {
        command.env("VOLANT_PYTHON", python);
    }
    let out = process::run_within(&mut command, std::time::Duration::from_secs(120), None);
    (
        out.status.code().expect("the run ends with a code"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A template that renders to a host name runs the play there, says nothing on stderr and
/// leaves the other hosts alone.
#[rstest]
#[case::first_of_a_group("\"{{ (groups['servers'] | first) }}\"", &[], "ran s1", "PLAY [s1]")]
#[case::extra_variable("\"{{ target | default('all') }}\"", &["-e", "target=w1"], "ran w1", "PLAY [w1]")]
#[case::list_result("\"{{ groups['servers'] | default([]) }}\"", &[], "ran s2", "PLAY [['s1', 's2']]")]
#[case::constant("\"{{ 'w1' }}\"", &[], "ran w1", "PLAY [w1]")]
#[case::colon_join("\"{{ groups['servers'] | join(':') }}\"", &[], "ran s2", "PLAY [s1:s2]")]
#[case::list_of_templates("[\"{{ 's1' }}\", \"{{ 'w1' }}\"]", &[], "ran w1", "PLAY [{{ 's1' }},{{ 'w1' }}]")]
fn a_template_that_renders_to_a_host_runs_the_play_there(
    #[case] hosts: &str,
    #[case] args: &[&str],
    #[case] ran: &str,
    #[case] banner: &str,
) {
    let (code, stdout, stderr) = run("runs", &play(hosts), args);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains(ran), "{stdout}");
    assert!(stdout.contains(banner), "{stdout}");
    assert!(!stderr.contains("WARNING"), "{stderr}");
}

/// `--limit` narrows what the template found, after the render.
#[test]
fn a_limit_applies_to_the_rendered_hosts() {
    let (code, stdout, stderr) = run(
        "limit",
        &play("\"{{ groups['servers'] }}\""),
        &["--limit", "s2"],
    );
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(
        stdout.contains("ran s2") && !stdout.contains("ran s1"),
        "{stdout}"
    );
}

/// What the render refuses stops the run before the banner with exit 4, and names the keyword:
/// `name` for a play that has none, because the reference renders the name first.
#[rstest]
#[case::undefined("\"{{ nosuchvar }}\"", "Error processing keyword 'name': ")]
#[case::undefined_hostvars("\"{{ hostvars['s1']['x'] }}\"", "Error processing keyword 'name': ")]
#[case::an_inventory_variable("\"{{ ansible_connection }}\"", "Error processing keyword 'name': ")]
#[case::an_integer(
    "\"{{ 3 }}\"",
    "Keyword 'hosts' items must be of type 'str', not 'int'."
)]
#[case::a_boolean(
    "\"{{ true }}\"",
    "Keyword 'hosts' items must be of type 'str', not 'bool'."
)]
#[case::a_mapping(
    "\"{{ {'a': 1} }}\"",
    "Keyword 'hosts' items must be of type 'str', not 'dict'."
)]
#[case::empty(
    "\"{{ '' }}\"",
    "Keyword 'hosts' is required, and cannot have empty values."
)]
#[case::a_list_in_a_list(
    "[\"{{ groups['servers'] }}\", \"w1\"]",
    "Keyword 'hosts' items must be of type 'str', not 'list'."
)]
fn a_template_the_render_refuses_exits_4(#[case] hosts: &str, #[case] message: &str) {
    let (code, stdout, stderr) = run("refused", &play(hosts), &[]);
    assert_eq!(code, 4, "{stdout}\n{stderr}");
    assert!(stderr.contains(message), "{stderr}");
    assert!(!stdout.contains("PLAY ["), "nothing ran: {stdout}");
}

/// A named play reports the keyword `hosts`.
#[test]
fn a_named_play_reports_the_keyword_hosts() {
    let playbook = format!(
        "- name: Named\n{}",
        play("\"{{ nosuchvar }}\"").replacen("- hosts:", "  hosts:", 1)
    );
    let (code, stdout, stderr) = run("named", &playbook, &[]);
    assert_eq!(code, 4, "{stdout}\n{stderr}");
    assert!(
        stderr.contains("Error processing keyword 'hosts': "),
        "{stderr}"
    );
}

/// A result with nothing in it is no error: an empty list matches no host and says nothing, and
/// `none` is the pattern `None`, which the reference warns about.
#[rstest]
#[case::empty_list("\"{{ [] }}\"", false)]
#[case::missing_group("\"{{ groups['nosuchgroup'] | default([]) }}\"", false)]
#[case::none("\"{{ none }}\"", true)]
fn a_template_that_finds_no_host_skips_the_play(#[case] hosts: &str, #[case] warns: bool) {
    let (code, stdout, stderr) = run("nothing", &play(hosts), &[]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("skipping: no hosts matched"), "{stdout}");
    assert_eq!(
        stderr.contains("Could not match supplied host pattern, ignoring: None"),
        warns,
        "{stderr}"
    );
}

/// The listings render too: the rendered pattern and the hosts it found, and the same refusal.
#[rstest]
#[case::hosts(&["--list-hosts"], "pattern: ['s1']\n    hosts (1):\n      s1")]
#[case::tasks(&["--list-tasks"], "play #1 (s1): s1")]
fn a_listing_renders_the_hosts(#[case] args: &[&str], #[case] shown: &str) {
    let (code, stdout, stderr) = run(
        "listing",
        &play("\"{{ groups['servers'] | first }}\""),
        args,
    );
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains(shown), "{stdout}");
}

#[rstest]
#[case::hosts("--list-hosts")]
#[case::tasks("--list-tasks")]
#[case::syntax("--syntax-check")]
fn a_listing_refuses_what_the_run_refuses(#[case] flag: &str) {
    let (code, stdout, stderr) = run("listing-refused", &play("\"{{ nosuchvar }}\""), &[flag]);
    assert_eq!(code, 4, "{stdout}\n{stderr}");
    assert!(
        stderr.contains("Error processing keyword 'name': "),
        "{stderr}"
    );
}

/// Text a host sent back is data. A registered result that holds a template is a pattern of
/// its own words, matches nothing, and is not evaluated.
#[test]
fn a_registered_result_that_holds_a_template_is_not_evaluated() {
    let playbook = "- hosts: s1\n  gather_facts: false\n  tasks:\n    - shell: printf '%s%s' '{' '{\"w1\"}}'\n      register: braces\n\n- hosts: \"{{ hostvars['s1']['braces']['stdout'] }}\"\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: never\n";
    let (code, stdout, stderr) = run("data", playbook, &[]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("skipping: no hosts matched"), "{stdout}");
    assert!(!stdout.contains("never"), "{stdout}");
    assert!(
        stderr.contains("Could not match supplied host pattern, ignoring: {{\"w1\"}}"),
        "{stderr}"
    );
}

/// A play 2 that reads the inventory's names, after a play 1 whose `add_host` took its host name
/// from a command's output. The output holds a template that touches `marker` when it is
/// evaluated. A host name from host text is data, whichever way the pattern reads it.
fn add_host_playbook(marker: &std::path::Path, hosts: &str) -> String {
    format!(
        "- hosts: s1\n  gather_facts: false\n  tasks:\n    - shell: printf '%s%s' '{{' '{{lookup(\"pipe\",\"touch${{IFS}}{}\")}}}}'\n      register: r\n    - add_host:\n        name: \"{{{{ r.stdout }}}}\"\n        groups: made\n        ansible_connection: local\n\n- hosts: \"{hosts}\"\n  gather_facts: false\n  tasks:\n    - debug:\n        msg: second\n",
        marker.display()
    )
}

#[rstest]
#[case::groups("{{ groups['all'] | join(',') }}")]
#[case::group_made("{{ groups['made'] | join(',') }}")]
#[case::play_hosts_all("{{ ansible_play_hosts_all | join(',') }}")]
#[case::play_hosts("{{ ansible_play_hosts | join(',') }}")]
#[case::hostvars("{{ hostvars | list | join(',') }}")]
#[case::lookup("{{ lookup('inventory_hostnames', 'all') | join(',') }}")]
fn a_host_name_made_from_host_text_is_not_evaluated(#[case] hosts: &str) {
    let Some(python) = python::reference_python() else {
        return;
    };
    let dir = fresh("added", "");
    let marker = dir.join("PWN");
    std::fs::write(dir.join("site.yml"), add_host_playbook(&marker, hosts)).expect("written");
    let (code, stdout, stderr) = run_in(&dir, &[], Some(&python));
    let evaluated = marker.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !evaluated,
        "the name was evaluated ({code}):\n{stdout}\n{stderr}"
    );
}

/// The same play 2, written by the author with a template that runs a command: it runs, which
/// shows the test above would see a marker.
#[test]
fn a_template_the_author_wrote_in_hosts_is_evaluated() {
    let dir = fresh("author", "");
    let marker = dir.join("PWN");
    let playbook = format!(
        "- hosts: \"{{{{ lookup('pipe', 'touch {}') }}}}\"\n  gather_facts: false\n  tasks: []\n",
        marker.display()
    );
    std::fs::write(dir.join("site.yml"), playbook).expect("written");
    let (code, stdout, stderr) = run_in(&dir, &[], None);
    let made = marker.exists();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        made,
        "the author's template did not run ({code}):\n{stdout}\n{stderr}"
    );
}

/// A pattern made of host text is a pattern, whatever its subscript says: a range that ends at the
/// largest integer used to overflow in a debug build.
#[rstest]
#[case::end_is_the_largest_integer("servers[0:9223372036854775807]", "ran s1")]
#[case::deprecated_form("servers[1-9223372036854775807]", "ran s2")]
fn a_huge_subscript_selects_what_it_covers(#[case] hosts: &str, #[case] ran: &str) {
    let (code, stdout, stderr) = run("subscript", &play(hosts), &[]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains(ran), "{stdout}");
}

/// A play named with an empty string prints the banner the engine always printed for it.
#[test]
fn an_empty_name_keeps_its_banner() {
    let playbook = "- name: \"\"\n  hosts: s1\n  gather_facts: false\n  tasks: []\n";
    let (code, stdout, stderr) = run("empty-name", playbook, &[]);
    assert_eq!(code, 0, "{stdout}\n{stderr}");
    assert!(stdout.contains("PLAY [] ***"), "{stdout}");
}

/// Variables that name each other in a ring never settle: the reference refuses the play.
#[test]
fn variables_that_name_each_other_are_refused() {
    let playbook = "- hosts: \"{{ a }}\"\n  gather_facts: false\n  vars:\n    a: \"{{ b }}\"\n    b: \"{{ a }}\"\n  tasks: []\n";
    let (code, stdout, stderr) = run("ring", playbook, &[]);
    assert_eq!(code, 4, "{stdout}\n{stderr}");
    assert!(!stdout.contains("PLAY ["), "{stdout}");
}

/// Under a terminal, a pause that waits for a prompt and a time is refused before the first
/// connection, whether the hosts are written out or come from a template.
#[cfg(target_os = "linux")]
#[rstest]
#[case::literal("s1")]
#[case::templated("{{ groups['servers'] | first }}")]
fn a_pause_that_cannot_wait_is_refused_before_the_run(#[case] hosts: &str) {
    let playbook = format!(
        "- hosts: \"{hosts}\"\n  gather_facts: false\n  tasks:\n    - pause:\n        prompt: go\n        seconds: 1\n"
    );
    let dir = fresh("pause", &playbook);
    let command = format!(
        "'{}' playbook -i inventory.ini site.yml",
        env!("CARGO_BIN_EXE_volant")
    );
    let mut script = Command::new("script");
    script
        .args(["-qec", &command, "/dev/null"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("VOLANT_RUNTIME", "system");
    let out = process::run_within(&mut script, std::time::Duration::from_secs(60), None);
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout).replace('\r', "");
    assert!(text.contains("cannot wait for a prompt"), "{text}");
    assert!(
        !text.contains("PLAY ["),
        "refused before the first banner: {text}"
    );
}
