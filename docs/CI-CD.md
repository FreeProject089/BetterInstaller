# CI/CD and security scans

Every GitHub Actions workflow in this repository: what starts it, what it checks, what makes it fail,
what it leaves behind, and how to run the same thing on your machine. The second half covers the
security scans: running each one by hand, changing thresholds, excluding a path, and dismissing a
false positive.

## The three workflows

| Workflow | File | Starts on | Blocks on |
|---|---|---|---|
| CI | `.github/workflows/ci.yml` | push to `main` / `master`, every pull request | formatting, a clippy warning, a failing test or build, an unexplained cargo advisory |
| Docs | `.github/workflows/docs.yml` | push to `master` or a pull request that touches `docs/`, `mkdocs.yml`, `requirements.txt` or the workflow; by hand | a broken link, missing locale, broken accented anchors, a truncated PDF |
| Security | `.github/workflows/security.yml` | push to `main` / `master`, every pull request, Monday 04:23 UTC, by hand | a secret in the history, or a Semgrep / Trivy finding at or above the threshold |

Every action is pinned to a full commit SHA with its tag in a comment, and every Docker image the
security scans use is pinned to a digest. Each workflow starts from `permissions: {}` or
`contents: read`, and a job gets more only when it writes something. This repository is public, so
a fork's pull request runs the same workflows with a read-only token.

### CI (`ci.yml`)

| Job | What it checks |
|---|---|
| build & test (Windows, Ubuntu) | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --release` |
| cargo audit | `node .github/scripts/dep-audit.mjs cargo .`: any RustSec vulnerability fails unless `.github/audit-ignore.json` explains it (advisory, folder, reason) |

- **Secrets / variables:** none. **Artifacts:** none.
- **Run it locally:** `docker compose run --rm ci` runs the whole Linux gate in the dev image (see the
  `Dockerfile`); `cargo install cargo-audit --locked` then `node .github/scripts/dep-audit.mjs cargo .`.
- **The dev image** is `FROM rust:<version>-trixie@sha256:<digest>`, pinned by version and digest
  (1.98.1 today, the `stable` that `ci.yml` installs). Bump tag and digest together, to the version
  CI's `stable` currently gives; the Dockerfile has the two commands. It runs as the unprivileged
  user `dev` (uid/gid 1000 by default, `--build-arg UID=… GID=…` or `BI_UID` / `BI_GID` with compose
  to match yours). The toolchain is left as the rust image ships it; the cargo registry and git caches
  and the target directory are created in the image owned by `dev`, so the named volumes mounted
  over them are too. Volumes created by the older, root-run image stay root-owned: on
  "Permission denied" under `/usr/local/cargo` or `/tmp/target`, run `docker compose down --volumes`
  once.

### Docs (`docs.yml`)

Builds the bilingual site with `mkdocs build --strict`, checks that accented French anchors survive,
checks that both locales built, then builds the PDF and reads it back with `tools/check_pdf.py`.
On `master`, the `deploy` job publishes to GitHub Pages.

- **Variables:** `PAGES_ENABLED=true` turns on the deploy job (after enabling Pages with the source
  set to GitHub Actions). Without it, the site is built and checked, not published.
- **Secrets:** none.
- **Artifacts:** `betterinstaller-pdf`.
- **Run it locally:** `pip install -r requirements.txt`, then `mkdocs build --strict` and
  `mkdocs build -f mkdocs.pdf.yml`. The PDF needs the pango and harfbuzz libraries listed in the
  workflow.

### Security (`security.yml`)

| Job | Tool | What it reads | Fails when |
|---|---|---|---|
| Gate self-test | `node --test` | `.github/scripts/security-gate.test.mjs` | the gate itself is wrong; every scan waits for it |
| Secrets | Gitleaks 8.30.1 | the whole git history, every branch | any finding not reviewed in `.gitleaksignore` / `.gitleaks.toml`, or zero commits read |
| SAST | Semgrep 1.178.0 | the whole checkout: `crates/`, `tools/`, the workflows | a finding at or above the Semgrep threshold |
| Dependencies + container config | Trivy 0.74.0 | `requirements.txt` (the docs and PDF toolchain) and the `Dockerfile` | a finding at or above the Trivy threshold |
| Upload SARIF to code scanning | `codeql-action/upload-sarif` | the three SARIF files | an upload that fails on a repository that supports code scanning |
| PR comment | `gh api` | the three verdicts | never fails a pull request by itself |

Each scanner writes its own report and exits 0. **One script decides:**
`.github/scripts/security-gate.mjs`, the same file in BetterInstaller, BMM, BMM Docs and BCW. It puts
every finding on one scale (`critical > high > medium > low > info`), prints a table and fails the job
only at or above the threshold.

- **Secrets:** none to create. The upload and comment jobs use the automatic `GITHUB_TOKEN`.
- **Variables:** see [Thresholds](#thresholds).
- **Artifacts** (kept 30 days): `security-gitleaks`, `security-semgrep` and `security-trivy`. Each
  holds the native JSON report, the SARIF, a `*-summary.md` table and a `*.result.json` verdict.

**Why some scans are not here:**

- **No DAST (OWASP ZAP, Nuclei).** BetterInstaller is a native installer and a CLI. There is no web
  application to scan, staging or otherwise; the docs are static pages on GitHub Pages. No job
  targets a host on the network.
- **No Trivy scan of `Cargo.lock`.** The `cargo audit` job in `ci.yml` already gates it, blocking,
  with reasons in `.github/audit-ignore.json`. A second scanner would need every accepted advisory
  written in two ignore files that drift apart.
- **No image scan.** No image is built or published. The `Dockerfile` is a local development image
  (`FROM rust`, pinned by version and digest). Its configuration is scanned; its packages are Debian's
  and the Rust toolchain's, in an image nobody ships.

## Reading the results

- **Job log and summary.** The gate prints one table per tool: severity, `BLOCK` or `pass`, rule id
  and location. It also writes that table to the job summary, and each blocking finding is an
  `::error` annotation. Gitleaks runs with `--redact`: the log shows rule, file, line, commit and
  fingerprint, never the value.
- **Pull-request comment.** The `PR comment` job posts one comment and edits that same comment on
  every later run. It finds it by the hidden first line `<!-- betterinstaller-security-gate -->`.
  The comment shows one row per tool (threshold, count per severity, blocking count, pass or
  **FAIL**) and an overall `PASSED`, `FAILED` or `INCOMPLETE`. `INCOMPLETE` means one scan produced
  no verdict; it is never shown as a pass. It ends with a link to the run and its artifacts. A fork's
  or Dependabot's pull request gets a read-only token, so the job prints a notice and posts nothing.
- **Code scanning (Security tab).** The Gitleaks, Semgrep and Trivy SARIF files are uploaded with
  one category per tool (`gitleaks`, `semgrep`, `trivy`). GitHub then tracks each alert over time
  and annotates the changed lines of a pull request. The upload runs even when a gate failed. The job
  first asks the API whether code scanning is available. This repository is public, so it is. On a
  private repository without GitHub Advanced Security, the job prints a notice and the SARIF stays
  in the artifacts.

Code scanning is a view. **The gate is what fails the build.** An alert dismissed in the Security tab
fails the job again at the next run, as long as the tool reports it.

## Thresholds

The threshold is the **lowest severity that fails**. Set it as a repository variable in
**Settings > Secrets and variables > Actions > Variables**:

| Variable | Values | Default |
|---|---|---|
| `SECURITY_GATE_SEVERITY` | `critical`, `high`, `medium`, `low` | `high` |
| `SECURITY_GATE_SEVERITY_SEMGREP` | same | the global value |
| `SECURITY_GATE_SEVERITY_TRIVY` | same | the global value |

- `critical` and `high` block by default. `medium` blocks only at `medium` or `low`. `low` blocks
  only at `low`. `info` never blocks.
- Gitleaks has no threshold: any secret that is not a reviewed exclusion fails.
- A value the gate does not know, such as a typo, **fails the run** (exit code 2). A typo must never
  mean that nothing blocks.
- Trivy's `UNKNOWN` counts as `high`. Semgrep's `ERROR`, `WARNING` and `INFO` count as `high`,
  `medium` and `low`.

## Running each scan by hand

From the repository root, with Docker and the same pinned images as CI. On Windows, use Git Bash
with `export MSYS_NO_PATHCONV=1`, or a clone inside WSL, and run from a fresh clone: Trivy walks the
whole folder, and a local `target/` is enough to reach its 5-minute timeout.

```bash
GITLEAKS=zricethezav/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f
SEMGREP=semgrep/semgrep:1.178.0@sha256:32e459968daabe7ab86968184a29109b9564aa00392401156f9788452b42786b
TRIVY=aquasec/trivy:0.74.0@sha256:62b1e65e8869bc4b4c6aa4fa2b21595256c7c2f6018a9d9ad61caf87187c1969
mkdir -p reports

# Secrets: the whole history. The log must say "N commits scanned" with N above 0.
docker run --rm -v "$PWD:/repo" \
  -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='*' \
  "$GITLEAKS" git /repo --redact --verbose \
  --report-format json --report-path /repo/reports/gitleaks.json --exit-code 0
node .github/scripts/security-gate.mjs --tool gitleaks --report reports/gitleaks.json

# SAST, with the rules at the pinned commit.
REF=$(grep -oE 'SEMGREP_RULES_REF: [0-9a-f]{40}' .github/workflows/security.yml | cut -d' ' -f2)
git init -q ../semgrep-rules && git -C ../semgrep-rules fetch -q --depth 1 https://github.com/semgrep/semgrep-rules "$REF" \
  && git -C ../semgrep-rules checkout -q FETCH_HEAD
CFG=$(grep -v '^#' .github/security/semgrep-rules.txt | grep . | sed 's#^#--config=/rules/#')
docker run --rm -v "$PWD:/src" -v "$PWD/../semgrep-rules:/rules:ro" -w /src \
  -e GIT_CONFIG_COUNT=1 -e GIT_CONFIG_KEY_0=safe.directory -e GIT_CONFIG_VALUE_0='*' \
  "$SEMGREP" semgrep scan --metrics=off $CFG --json-output=reports/semgrep.json .
node .github/scripts/security-gate.mjs --tool semgrep --report reports/semgrep.json

# Dependencies and the Dockerfile.
docker run --rm -v "$PWD:/src" -w /src "$TRIVY" fs --scanners vuln,misconfig --show-suppressed \
  --ignorefile .github/security/trivyignore.yaml \
  --skip-files Cargo.lock --format json --output reports/trivy.json --exit-code 0 .
node .github/scripts/security-gate.mjs --tool trivy --report reports/trivy.json

# The gate itself.
node --test .github/scripts/security-gate.test.mjs
```

To render the pull-request comment locally, add `--json reports/<tool>.result.json` to each gate
line, then run `node .github/scripts/security-gate.mjs --markdown --results reports/gitleaks.result.json --results reports/semgrep.result.json --results reports/trivy.result.json`.

To try another threshold on a downloaded artifact:
`SECURITY_GATE_SEVERITY=medium node .github/scripts/security-gate.mjs --tool trivy --report trivy.json`.

## Adding or excluding a target

| To… | Edit |
|---|---|
| Add a Semgrep rule | a line in `.github/security/semgrep-rules.txt`: a rule file path in `semgrep/semgrep-rules` at the pinned commit |
| Exclude paths from Semgrep | a `.semgrepignore` at the root, one pattern per line, with a comment saying why |
| Give a new lockfile to Trivy | nothing: `trivy fs` finds every lockfile except `Cargo.lock` |
| Exclude a folder from Trivy | `--skip-dirs <dir>` in the `trivy fs` step, with a comment saying why |
| Bump the Semgrep rules | change `SEMGREP_RULES_REF`, regenerate the list and read the diff |

The rule list holds only rule files whose rules are not tagged `subcategory: audit`. Audit rules flag
every use of a sink for a human to review. They are a manual-review aid, not something a CI gate can
decide on. Rules from a pinned commit give the same result next year, while a registry ruleset
(`p/…`) changes under you. The cost is that new rules arrive only when someone bumps the ref.

## Dismissing a false positive properly

**Prefer the tool's own config** over a dismissal in the Security tab. The config is versioned,
reviewed in a pull request, carries its reason, and applies to the gate, the artifacts and the
Security tab alike. A Security-tab dismissal only hides the alert in that view; the gate still fails
at the next run.

| Tool | Where | Format |
|---|---|---|
| Gitleaks, one finding | `.gitleaksignore` | the fingerprint from the log (`commit:file:rule:line`), under a comment saying why |
| Gitleaks, a class of false positive | `.gitleaks.toml` with `[extend] useDefault = true` and `[[allowlists]]` | `targetRules` + a narrow `regexes`, with a comment proving it cannot match a real secret |
| Semgrep | the source line | `// nosemgrep: <rule-id>` (`#` in Python), with the reason on the line above |
| Trivy | `.github/security/trivyignore.yaml` (the workflow passes `--ignorefile`; Trivy reads no YAML file on its own) | an entry under `vulnerabilities:` or `misconfigurations:` with `id`, `paths` (as narrow as possible), `statement` (the reason) and `expired_at`; the expiry makes the finding fail again, for review |
| cargo audit (`ci.yml`) | `.github/audit-ignore.json` | id, tool, folder, reason, date |

- **A real secret is never excluded.** Rotate it first. Only then, if it must stay in the history,
  add its fingerprint with the rotation date.
- An exclusion names one finding or one provably harmless class, never a whole rule or folder "to make
  CI pass".
- A change to any of these files is a security change: review it like one.

**Current exclusions:** one. Trivy `DS-0026` (no `HEALTHCHECK`, LOW), scoped to `Dockerfile`, expires
2027-03-25: the dev image is a one-shot container that runs the gate and exits, with no long-running
service to probe. `--show-suppressed` keeps it listed as "ignored" in every run.

A Security-tab dismissal (`False positive` / `Won't fix`) is still the place to record a decision
the tool config cannot express. Write the justification in the comment, not just the reason code.

## Testing without touching anything live

- Every scan reads only the checkout. No job connects to an update server, a user's machine or any
  production host. The only network traffic fetches the pinned images, Trivy's advisory database and
  the pinned Semgrep rules.
- Run the workflows on a branch or a pull request, or with **Run workflow** (`workflow_dispatch`).
- Run the commands above locally; the reports land in `reports/`. Do not commit that folder.
- `node --test .github/scripts/security-gate.test.mjs` proves the gate still tells `high` from `low`,
  and treats a missing report or a scan of nothing as a failure.
