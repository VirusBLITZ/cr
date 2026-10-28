# cr

Review and fix GitHub pull requests with the `codex` and `claude` CLIs.

## Install

Prerequisites (Arch):

```sh
sudo pacman -S claude-code openai-codex github-cli
gh auth login   # and sign in to claude and codex once
```

Then:

```sh
cargo install --git https://github.com/VirusBLITZ/cr
# or, from a clone:
cargo install --path .
```

## Usage

Run inside the PR's repository. `TARGET` is a PR URL, PR number or branch name; omit it to use the current branch's PR.

```sh
cr local             # alias: cr l — review + fix local work, no PR needed
cr review [TARGET]   # alias: cr r
cr fix [TARGET]      # alias: cr f
cr init              # write the default config
```

- **`cr local`** reviews your work before a PR exists: everything that differs from the base branch, committed and uncommitted. It merges the findings and lets the fixer handle them. Nothing is committed, pushed or posted, so check `git diff` afterwards. `--base <ref>` picks the base (default: origin's default branch, else `main`/`master`); `--no-fix` only reviews. Works in any git repo, GitHub or not.

- **`cr review`** checks out and pulls the PR branch, runs the codex and claude reviews (plus codex's `ponytail-review` skill, if installed) in parallel with a live status display, merges their findings into one deduplicated list, and posts it as a PR comment. `--no-comment` only prints it; `--reviewers codex` runs a subset.
- **`cr fix`** reads the latest cr review plus all other PR comments, lets an agent validate and fix each one and run the project's checks, then commits, pushes and posts a status table. The comment only says green when every item was handled and every check passed. `--no-push` leaves the fixes uncommitted.

Inside a VS Code terminal, the formatted report (`report.md`) also opens in the editor once the review is done; `cr local` adds the fix summary to it when the fixer finishes. To have it open as a rendered preview, add this to your VS Code settings:

```json
"workbench.editorAssociations": { "**/.git/cr/**/report.md": "vscode.markdown.preview.editor" }
```

`review` and `fix` refuse to run on a dirty working tree. Raw reviewer output is kept in `.git/cr/<pr>/` (`.git/cr/local/` for `cr local`).

## Configuration

`cr init` writes `~/.config/cr/config.toml`; `cr init --repo` writes a `.cr.toml` that overrides it per repository.

```toml
auto_comment = true

[[reviewers]]
tool = "codex"            # codex | claude

[[reviewers]]
tool = "claude"
model = "opus"            # omit to use the CLI's own default

[[reviewers]]
tool = "codex"
skill = "ponytail-review" # review with a skill; skipped if not installed

[merger]
tool = "codex"
model = "gpt-6-luna"

[fixer]
tool = "claude"
args = ["--permission-mode", "bypassPermissions"]   # passed verbatim
```

By default the merger uses `gpt-6-luna`; everything else uses each CLI's selected model.
