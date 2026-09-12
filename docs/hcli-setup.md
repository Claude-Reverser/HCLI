# HCLI startup

Start the app with no subcommand to set up hcap.ai. The first screen asks for
your API key from <https://hcap.ai>, masking it as you type. Empty input asks
again; Ctrl+C cancels setup. Later launches reuse the saved key. For unattended
use, provide `HCAP_API_KEY` in the environment.

Setup creates an `hcap` OpenAI-compatible provider profile using
`https://hcap.ai/v1`, with `gpt-6-astra` as the initial model and model
discovery through `/v1/models`. Use `--model` to choose a different model.
Saving a key does not validate it or make a billable model request.

HCLI defaults to `~/.hcli` for application state and uses `hcli.sock` in an
`hcli` subdirectory of the platform runtime directory. This also isolates the
daemon lock from Jcode. The key is stored separately from `config.toml`,
in `~/.hcli/config/jcode/hcap.env`, with owner-only file permissions. An
environment key takes precedence and is not persisted automatically.
`JCODE_HOME`, `JCODE_RUNTIME_DIR`, `JCODE_SOCKET`, and `--socket` overrides remain supported.

If the launching shell's directory was deleted (for example by an app update),
HCLI reports the problem and opens in your home directory. Use `-C /path/to/project`
to select a workspace explicitly. The hcap.ai launch path skips the upstream
Jcode desktop launcher and global hotkey installation.

The TUI uses a slate/cyan palette, a compact workspace header, and a rounded
message box anchored at the bottom of the terminal. Conversation history fills
the available height. Existing `/colors` overrides still take precedence; set
`NO_COLOR` for plain output.

Below the message box, the footer shows context used/limit, context percentage,
measured output tokens/sec, reasoning effort when available, catalog input/output
prices per million tokens, and cumulative session input/output tokens. Narrow
terminals omit optional metrics to keep the footer within the window. `~` marks
estimated context usage; `—` means a value is not available yet. Speed prefixed
with `~` estimates tokens from streamed characters (four characters per token)
until reported usage arrives. The final rate stays visible until the next turn,
using generation time only, excluding tool execution and initial waiting. It is
independent of the catalog's nullable speed field. hcap's
nested `extra.context` and USD-per-million prices are read through the existing
background model-catalog refresh, so model switches can use their own metadata.

Explicit provider selections, SSH sessions, simulator modes, and CLI
subcommands keep their existing flows. Noninteractive default launches without
a key fail with instructions to set `HCAP_API_KEY`.

The build target and executable are still named `jcode` during this first step
of the HCLI fork. For a local build:

```sh
cargo +1.91.0 build --profile selfdev --locked --no-default-features --bin jcode
./target/selfdev/jcode --no-update --no-selfdev
```

The `HCLI Builds` GitHub Actions workflow builds macOS, Linux, and Windows for
x64 and ARM64, plus FreeBSD x64. Download an artifact from a completed workflow
run, then extract the archive inside it. Each archive includes `hcli` (or
`hcli.exe`), this guide, the license, and a `BUILD.json` recording its commit.
The adjacent `.sha256` file verifies the archive. Run from your project folder:

```sh
./hcli --no-update --no-selfdev
```

On Windows, use `.\hcli.exe --no-update --no-selfdev` in PowerShell. These
development builds are unsigned and use `--no-default-features`, matching the
tested HCLI build; optional local embeddings, PDF parsing, and AWS Bedrock are
omitted. Linux x64 is built on Ubuntu 22.04 and ARM64 on Ubuntu 24.04; these
are glibc builds, not Alpine/musl or Android/Termux packages. FreeBSD builds
target FreeBSD 15.1. macOS builds are not notarized.

Run the complete startup regression check against a fresh executable:

```sh
python3 tests/test_hcli_startup.py ./target/selfdev/jcode
```

It uses a local mock API, temporary credentials, and an isolated daemon to check
that fresh and returning launches reach a connected TUI, including deleted
working directories and relative `-C` paths.

Command suggestions appear in a bordered, shaded dropdown with row separators
and keyboard hints. Typing `/model` or `/models` opens a split model library;
add a space and a search term to filter, use ↑/↓ to inspect, and Enter to select.
The details pane follows the highlighted route and shows hcap catalog context,
maximum output, input/output pricing, wallet, catalog speed, capabilities,
variant, request count, and last-used timestamp. Catalog activity is provider
metadata, not your session usage. Missing fields stay unknown. At narrow widths,
the inspector stacks below the list; larger windows show it beside the list.
