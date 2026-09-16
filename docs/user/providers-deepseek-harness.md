# DeepSeek Harness

DeepSeek Harness connects T3 Code to the `dsh` agent through ACP.

## Before You Start

Install DeepSeek Harness on the computer that runs your T3 server. The `dsh` command must be
available on that computer's `PATH`.

For a remote T3 environment, this means the host computer, not the phone or browser you use to
control it. T3 starts and supervises the process on the host.

## Add A Provider

1. Open **Settings**.
2. Select **Providers**.
3. Select **Add instance**.
4. Choose **DeepSeek Harness**.
5. Confirm the process settings and save the instance.

The defaults are:

```text
Command: dsh
Arguments:
--profile
acp
Working directory: empty
```

Use **Command** to select another `dsh` executable. Add one argument per line in **Arguments**. Set
**Working directory** only when the harness must start in a specific directory.

API keys and other secrets do not belong in Command or Arguments. Add them in the provider's
**Environment variables** section and mark sensitive values as sensitive.

## Current Limits

DeepSeek Harness does not currently accept attachments or support plan interaction mode in T3 Code.
Use a normal text prompt instead.

## If The Saved Package Is No Longer Installed

T3 pins each instance to an exact adapter package version and protocol. If the host no longer has
that exact package, the instance remains visible and its saved values are preserved. Its settings
are read-only, but you can still delete it.

Reinstall the exact package shown for the instance to make it editable again. Installing a different
version does not silently change the saved instance.
