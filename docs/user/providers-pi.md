# Pi

T3 Code can run the Pi coding agent through Pi's local RPC mode. T3 starts and supervises the Pi process on the server machine, then shows its sessions and output in the normal T3 interface.

## Install Pi

Install the Pi CLI on the machine that runs your T3 server:

```bash
npm install --global @earendil-works/pi-coding-agent
```

Confirm that the `pi` command works and complete any provider sign-in or API-key setup required by the models you want to use.

## Add Pi to T3 Code

1. Open **Settings**.
2. Select **Providers**.
3. Select **Add instance**.
4. Choose **Pi**.
5. Keep the default executable `pi`, or enter an absolute executable path if `pi` is not on the server's `PATH`. T3 adds Pi's RPC and session-storage arguments itself. Use **Arguments** only for additional safe Pi options.
6. Save the instance.

Environment variables configured on the instance are passed to Pi. Keep credentials there instead of putting them in adapter arguments. T3 rejects Pi session-selection flags because it owns session storage and validates every resume file.

## Supported behavior

The Pi connection supports:

- new sessions and native session resume;
- follow-up prompts and mid-turn steering;
- interrupts;
- provider-qualified model selection and thinking-level selection;
- reasoning, tool lifecycle, and token-usage streaming;
- Pi select, confirm, input, and editor questions;
- subagents launched by the `@mjakl/pi-subagent` extension in the Agents panel, with per-child status, model, activity, and reported token usage.

Named subagent sessions reuse their panel row when given more work. Parallel calls appear separately, and each child can finish or fail independently. This requires structured subagent progress from the extension; arbitrary tools named “subagent” do not create agent rows. Previously recorded tool-only activity is not converted into agent history.

T3 waits for Pi's `agent_settled` event before marking a turn complete. This keeps tool loops and retries inside one T3 turn.

Pi currently supports only **Full access** runtime mode. Attachments, T3 approval requests, and rollback are not currently advertised for Pi. Pi extension display updates do not become blocking questions in T3.
