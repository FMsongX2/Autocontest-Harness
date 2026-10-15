**English** · [한국어](SECURITY_ko.md)

# Security

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: **Security → Report a vulnerability** on https://github.com/FMsongX2/Autocontest-Harness. Please do not open a public issue for a vulnerability. Include the version (`harness --version`), the steps or input that trigger it, and what an attacker gains. You will get an answer within seven days.

## Threat model

The harness runs coding agents unattended on text it does not control: listing pages, official notices, organizer sites and emails. It also holds personal data (`data/profile.md`) and a GitHub login. The main risks and what the harness does about them:

| Risk | Mitigation | Limit |
|---|---|---|
| Shell injection through scraped titles, notes or receipts | Agents pass fetched text to the CLI through files (`add --from-file`, `--note-file`, `--receipt-file`); only ids (`[a-z0-9-]+`) appear on command lines. Collected items whose id is not lowercase alphanumeric are skipped. Titles are reduced to one line without control characters. The CLI runs `git`, `gh`, `orca` and `tmux` with argument lists, never through a shell | An agent can still ignore its instructions |
| Prompt injection from fetched pages | Every prompt treats fetched pages, listings, emails and registry text as data, not instructions, and tells the agent to report instructions found there instead of following them. Entry values in the launch prompt sit in a fenced data block | Models can still be manipulated; the CLI checks below are the backstop |
| Submitting or reading personal data without the user's consent | `harness submit` and `harness profile show --for <id>` require a recorded `harness approve <id>`. `approve` refuses to run inside a contest session (`HARNESS_SESSION` is set by `harness launch`), so a contest session cannot approve itself. Level 5 plan reviews move on only through `approve` | A session that clears the environment variable can bypass the check; approval is a safety catch, not a sandbox. Run sessions with your harness's own permission controls |
| Publishing private work | Contest repositories are created private with Actions disabled. `route` refuses an existing repository that is public, or that belongs to another contest. Session logs, launch prompts and `.private/` are git-ignored in contest folders | Making a repository public for a submission is a manual step |
| Leaking personal state from the harness checkout | `data/` holds personal state, is git-ignored, and is created with mode 0700 and files 0600. Release binaries are built with local paths removed | Back up `data/` yourself |
| Path traversal in cleanup | `clean` archives only paths inside the contest folder and moves only folders that `route` created | |

<p>Table 1. Risks and mitigations</p>
