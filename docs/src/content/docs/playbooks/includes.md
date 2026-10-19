---
title: Includes and imports
description: Static imports, dynamic includes, include_vars, and when each one is checked.
---

Five statements bring work in from another file. They fall into two families:

| Statement | Kind | When Volant reads the file | What a listing shows |
|---|---|---|---|
| `import_tasks` | static | while the play compiles | every task inside |
| `import_role` | static | while the play compiles | every task inside |
| `import_playbook` | static | while the play compiles | every play inside |
| `include_tasks` | dynamic | when a host reaches the statement | the statement only |
| `include_role` | dynamic | when a host reaches the statement | the statement only |

A dynamic include is resolved per host, because its file name can depend on that host's variables. The new steps are spliced in right behind the statement while the play runs.

## Static imports

An `import_tasks` path is looked for next to the importing file first, then in the role's own `tasks/` directory.

`import_playbook` splices the other file's plays where the statement stands, and each imported play keeps the directory of the file it came from. Its file name renders with no variables at all, so `{{ 'sub' }}.yml` works and `{{ nosuchvar }}.yml` does not.

Three keywords of the `import_playbook` entry reach every play of the file:

- `tags` are added to the tags of each imported play, so `--tags` and `--skip-tags` select its tasks and the listings show them.
- `vars` are added to the variables of each imported play and win over the play's own.
- `when` is put in front of the conditions of every task, role and block of each imported play, and in front of its fact gathering. It is not evaluated once for the whole import. A false condition still opens each play and still prints a skip for each task and host. Handlers are not conditional.

The other keywords that ansible-core 2.19 accepts on the entry, such as `ignore_errors`, `become` and `environment`, do nothing there, and Volant ignores them the same way. Any other key is refused. A `tags` or a `when` that holds a template is refused too, because Volant has no variables when it loads the playbook.

## Dynamic includes

Of what the statement itself says, only its `vars:` travel into the included file. Those variables enter at the role-parameter level of the [precedence order](/variables/precedence/), above facts and below `--extra-vars`, and they accumulate through nested includes. The statement's own `tags` stop at the statement, and its `when` is evaluated for the statement alone.

What the layers around the statement say does reach the included tasks: the play, the role entry that brought the file in, and the blocks around it. Their `tags`, `become`, `become_user`, `no_log`, `when`, `environment`, `delegate_to`, `ignore_errors`, `notify` and `vars` apply to every included task, and a task's own value wins over an inherited one. This is ansible-core 2.19's rule, which skips a dynamically loaded parent and asks the grandparent. An inherited `check_mode: true` is refused in the included file rather than dropped.

An included task lands in the block and section where the statement was written, so a failing include inside a block body is caught by that block's `rescue`.

Volant checks what an include brings in before it runs. By then the hosts are connected and earlier tasks have run, so a module or keyword this release does not support yet can no longer stop the whole run. It fails the include statement for the host that reached it instead, nothing in the file runs, and a `rescue` around the statement can catch it.

The Python modules an included file names are found earlier. The union of Python modules is built before the first connection from every task and handler file of the roles in play, so a module named only in `setup-{{ ansible_os_family }}.yml` is already on the host when the include runs. A file under a role's `tasks/` or `handlers/` that does not parse stops the run before anything connects. See [The warm Python path](/internals/python/#how-it-works).

A `notify` in an included file that names no handler ends the run with exit code 1 and the pre-flight's sentence, as ansible-core does. No `ignore_errors` or `rescue` can catch it. See [Handlers](/playbooks/handlers/#differences-from-ansible).

## include_vars

`include_vars` produces variables rather than steps, so it is neither static nor dynamic in this sense. It is an ordinary controller-side module, and what it reads lands at facts precedence.

`include_vars` reads one file, named by `file:` or written as the free-form path, or every file of a directory, named by `dir:`. The two forms take different options, and giving options of both forms fails the task.

A file is looked for in the places `include_vars` searches: `vars/` and then the directory itself, for a role and then for the playbook. A `dir:` is not searched for. A task of a role reads it under the role's `vars/`, and `dir: vars/x` names a directory of the role itself when that exists. Any other task reads it against the directory of the file that holds the task. An absolute path is used as it is.

A directory is read in a fixed order: directories sorted by their full path as text, then the files of each directory sorted by name. Hidden files and directories are read, and links to directories are followed. A later file replaces the keys of an earlier one whole. A file with nothing in it adds nothing, and it is still listed in `ansible_included_var_files`.

| Option | Meaning |
|---|---|
| `dir` | The directory to read. |
| `depth` | How many levels to read, the directory being the first. `0`, the default, reads them all. |
| `files_matching` | A regular expression searched in each file name, not in its path. Files that do not match are left out. |
| `ignore_files` | A list of regular expressions, or one string of them split on white space. Each is anchored at the end of the name. Files that match are left out. |
| `extensions` | The extensions a file may have, without the dot. The default is `yaml`, `yml` and `json`. The comparison is case-sensitive. |
| `ignore_unknown_extensions` | Leave out a file with another extension. Without it, such a file fails the task. |
| `name` | Put everything under this one variable. It also works for a single file. |
| `hash_behaviour` | `merge` lays each variable over the variable of that name that the task already sees, mappings merged all the way down. Any other value replaces. |

The result holds `ansible_facts` and `ansible_included_var_files`, the files in the order they were read. A task that reads a vaulted file is hidden like a `no_log` task: `-v` shows the censored line, and a loop item that read one shows `(censored due to no_log)`. The task runs in check mode, as it does in Ansible.

A missing directory, a path that is not a directory, a file without a valid extension and a file that is not a mapping fail the task with `message` set, as in Ansible. A failure in a directory keeps the variables of the directories before the failing one in the result, but a failed task sets no variables. A file that cannot be parsed or decrypted raises the error instead, and the task has no result of its own. An option of the wrong type fails with Python's own sentence.

## Nesting limit

A statement 32 levels deep is rejected. One counter covers role nesting, `import_role` inside a role's tasks and `import_tasks` inside an imported file, because a cycle can run through any mix of the three.

## Differences from Ansible

- A file that includes itself hits that limit and is reported as a task failure with exit code 2. Ansible recurses until the Python stack runs out, at exit code 250.
- An included file outside a role's `tasks/` and `handlers/` that does not parse fails the task, with exit code 2 and a recap. Ansible exits with code 4 and no recap. A failure the recap accounts for is more useful, and a `rescue` can catch it.
- A failing include inside a loop prints the usual `failed: [host] (item=...)` line. Ansible prints only an `[ERROR]` on standard error.
- `include_role` does not support `allow_duplicates`, `rolespec_validate` and `apply` yet, and accepts `public: false` only, which is what Volant does. Honoring `public: true` would mean changing exported variables while the play's hosts stand at different steps.
- `include_vars: file=x.yml name=ns` is read as one raw argument rather than split into `key=value` pairs, the same limitation `import_tasks` has.
- What `include_vars` produces sits one level above where Ansible puts it. You only notice with a name that a `set_fact` also wrote: Ansible lets the `set_fact` win in either order, and here the later write wins.
- The text of a YAML syntax error in a file that `include_vars` reads is the parser's own, not ansible-core's `YAML parsing failed: ...`. The shape of the failure is the same.
- A failing `include_vars` has `"failed": true` in its result.
