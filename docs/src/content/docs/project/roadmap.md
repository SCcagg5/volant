---
title: Roadmap
description: What Volant is working toward, in the order it is likely to land.
---

Volant is in pre-alpha. This page lists what comes next, roughly in order. It describes intent, not dates, and the [changelog](https://github.com/lorica-labs/volant/blob/main/CHANGELOG.md) records what actually shipped.

## Next: the modules real roles use

- The [known gaps](/start/compatibility/#known-gaps) that stop real roles, such as a variable that reads another templated variable through a filter, and a `hosts:` pattern written as a template. Every module backed by an action plugin already runs, `uri` and `script` included, and so do modules from installed collections.
- The Jinja2 filters, tests and lookups that popular Galaxy roles still miss.

The goal is to run well-known Galaxy roles end to end and produce the same recap as `ansible-playbook`. Four already do, as published: [decision record 0007](/decisions/0007-action-plugins/#results) has the runs.

## Then: more speed

Native versions of the common modules, native fact gathering, a cache for the Python module union, one ssh connection per host and compressed blobs are in. The [performance page](/project/performance/) has the figures. What remains:

- `service_facts` and an `apt` task that refreshes its cache on every run, the two steps that hold the k3s workload back. Both are bound by commands the reference itself runs.
- Native versions of more modules, and wider subsets for the existing ones.

## Then: ready for daily use

- A `volant check` command that tells you whether a project will run, and what blocks it.
- A Homebrew formula.
- A nightly run over a corpus of public roles, compared with ansible-core.
- Video walkthroughs of the main workflows.

## Later

- Plugins of your own that run on the controller: action, callback and vars plugins.
- The collection action plugins for Windows hosts and network devices.
- Windows hosts and network devices.

Something missing that blocks you? [Open a feature request](https://github.com/lorica-labs/volant/issues/new?template=feature.yml) or start a thread in [Discussions](https://github.com/lorica-labs/volant/discussions).
