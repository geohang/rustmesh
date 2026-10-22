# Agent panel

The desktop app has an optional **Agent & workspace** panel. After you sign in with
ChatGPT, you describe a meshing task in words and the Codex command line tool
carries it out through the app's MCP tools, on the same project you see on the
canvas: it creates a project from a template, edits geometry and sizing, builds
meshes, reads their reports, compares them, undoes edits and exports files. The
mesher itself runs locally; the prompts and the tool results that go back to the
model are sent to the model provider. The agent code lives in
`crates/rustmesh-app/src/agent` and is not part of the mesher library or the web
build.

## Before signing in

- Build both `rustmesh-app` and `rustmesh` and keep them in the same folder; the
  agent reaches the workspace through `rustmesh mcp`.
- Install the Codex CLI. The app works with one version only,
  `codex-cli 0.163.0-alpha.10.2`, and says so in the panel when it finds another.
  Set `RUSTMESH_CODEX` to the full path of the executable, or put it on `PATH`.
  rustmesh does not install Codex or change its global configuration.

## Signing in

1. Open the panel and choose **Continue with ChatGPT**. To use an account you have
   signed in with before, choose its **Reconnect** button instead.
2. Finish OpenAI's sign-in and consent in the system browser, and approve ChatGPT
   plan usage for the account or workspace if asked. Return to rustmesh when the
   browser says it has the authorization.
3. Choose a model from the list the account returns, type a request, and send it.
   **Stop turn & jobs** interrupts the agent and cancels the mesh jobs it started.
   **Disconnect rustmesh session** ends the session; if OpenAI cannot be reached to
   revoke it, the panel says so and you can remove rustmesh in ChatGPT Settings.

Every project change the agent makes is a new revision, so **Undo last project
change** works on it too, and **Save offline workflow** writes the recorded
commands to a file that `rustmesh replay` can run later without any model.

## What is stored

Tokens are kept in memory only. The app saves its own host ID and the registration
(client ID, account subject and email) of each signed-in account under its local
data folder (`%LOCALAPPDATA%/rustmesh/agent` on Windows), and the Codex history in
a `codex-runtime` folder there. The model works in a separate empty folder, read-only,
with Codex's shell, web search and other built-in tools switched off; it reaches
the project only through the app's MCP tools. After a restart you sign in
again; the saved registration is reused. Each finished turn also writes a
`agent-turn-*.json` summary (status and number of tool calls, no credentials) into
the workspace folder.

## How sign-in works

rustmesh registers itself with OpenAI as its own client. It listens on a loopback
port before opening the browser, and each attempt has a new PKCE verifier, state
and nonce. The returned ID token's signature, issuer, audience, expiry, subject and
nonce are checked, and a returning registration must keep its client and subject.
The host ID is a persistent `urn:uuid:<UUIDv4>` URI. Running a model needs the
direct plan scope in the token response; identity alone is not enough. Refreshing
the token restarts the Codex process, and the next message resumes the saved thread
with the new token; unfinished work is not replayed. No other application's
credentials are read, and there is no fallback to another login or a billed API.

## Limits

- Sign-in works on the desktop only.
- Credentials are not stored between sessions.
- Only Codex and ChatGPT sign-in are supported.
- The model's work is bounded: a turn may start at most three mesh jobs.
- A turn can fail on a usage limit, a missing plan permission, a missing or
  unsupported Codex version, an expired session or a network error; the panel
  shows the reason.
