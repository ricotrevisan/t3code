import { afterEach, describe, expect, it } from "vite-plus/test";

import {
  loadDirenvExportedEnv,
  mergeDirenvExportedEnv,
  resetWorkspaceDirenvEnvCache,
} from "./workspaceDirenvEnv.ts";

afterEach(() => {
  resetWorkspaceDirenvEnvCache();
});

describe("loadDirenvExportedEnv", () => {
  it("returns an empty overlay when cwd is missing", () => {
    expect(loadDirenvExportedEnv(undefined)).toEqual({});
    expect(loadDirenvExportedEnv("  ")).toEqual({});
  });

  it("skips direnv when no .envrc exists in cwd or parents", () => {
    const warnings: string[] = [];
    const overlay = loadDirenvExportedEnv("/tmp/t3-direnv-no-envrc", {
      findEnvrc: () => undefined,
      resolveDirenvPath: () => "/usr/bin/direnv",
      runExport: () => {
        throw new Error("direnv should not run");
      },
      logger: (message) => warnings.push(message),
    });
    expect(overlay).toEqual({});
    expect(warnings).toEqual([]);
  });

  it("skips direnv when the binary is missing", () => {
    const warnings: string[] = [];
    const overlay = loadDirenvExportedEnv("/tmp/t3-direnv-project", {
      findEnvrc: () => "/tmp/t3-direnv-project/.envrc",
      resolveDirenvPath: () => undefined,
      logger: (message) => warnings.push(message),
    });
    expect(overlay).toEqual({});
    expect(warnings).toEqual([
      "direnv: skipped workspace env for '/tmp/t3-direnv-project' (direnv missing)",
    ]);
  });

  it("skips direnv when allow is not done and does not log export output", () => {
    const warnings: string[] = [];
    const overlay = loadDirenvExportedEnv("/tmp/t3-direnv-blocked", {
      findEnvrc: () => "/tmp/t3-direnv-blocked/.envrc",
      resolveDirenvPath: () => "/usr/bin/direnv",
      runExport: () => ({
        status: 1,
        stdout: '{"SECRET":"should-not-appear"}',
        stderr: "direnv: error /tmp/t3-direnv-blocked/.envrc is blocked. Run `direnv allow`",
      }),
      logger: (message) => warnings.push(message),
    });
    expect(overlay).toEqual({});
    expect(warnings).toEqual([
      "direnv: skipped workspace env for '/tmp/t3-direnv-blocked' (direnv allow not done)",
    ]);
    expect(warnings.join("\n")).not.toContain("should-not-appear");
    expect(warnings.join("\n")).not.toContain("SECRET");
  });

  it("skips when direnv exits 0 but stderr reports a missing command", () => {
    const warnings: string[] = [];
    const overlay = loadDirenvExportedEnv("/tmp/t3-direnv-missing-cmd", {
      findEnvrc: () => "/tmp/t3-direnv-missing-cmd/.envrc",
      resolveDirenvPath: () => "/usr/bin/direnv",
      runExport: () => ({
        status: 0,
        stdout: '{"DIRENV_DIR":"-/tmp/t3-direnv-missing-cmd"}',
        stderr: "./.envrc:1: varlock: command not found",
      }),
      logger: (message) => warnings.push(message),
    });
    expect(overlay).toEqual({});
    expect(warnings).toEqual([
      "direnv: skipped workspace env for '/tmp/t3-direnv-missing-cmd' (direnv export failed)",
    ]);
  });

  it("returns exported keys and strips DIRENV_ internals", () => {
    const overlay = loadDirenvExportedEnv("/tmp/t3-direnv-ok", {
      findEnvrc: () => "/tmp/t3-direnv-ok/.envrc",
      resolveDirenvPath: () => "/usr/bin/direnv",
      runExport: () => ({
        status: 0,
        stdout: JSON.stringify({
          OPENAI_API_KEY: "secret-value",
          ADMIN_EMAIL: "ops@example.com",
          DIRENV_DIR: "-/tmp/t3-direnv-ok",
          DIRENV_WATCHES: "blob",
        }),
        stderr: "",
      }),
      logger: () => {
        throw new Error("success should not warn");
      },
    });
    expect(overlay).toEqual({
      OPENAI_API_KEY: "secret-value",
      ADMIN_EMAIL: "ops@example.com",
    });
    expect(overlay.DIRENV_DIR).toBeUndefined();
  });

  it("caches overlays per resolved cwd so a directory change reloads", () => {
    let exports = 0;
    const options = {
      findEnvrc: (cwd: string) => `${cwd}/.envrc`,
      resolveDirenvPath: () => "/usr/bin/direnv",
      runExport: ({ cwd }: { cwd: string }) => {
        exports += 1;
        return {
          status: 0,
          stdout: JSON.stringify({ FROM: cwd }),
          stderr: "",
        };
      },
    };

    expect(loadDirenvExportedEnv("/tmp/t3-direnv-a", options).FROM).toBe("/tmp/t3-direnv-a");
    expect(loadDirenvExportedEnv("/tmp/t3-direnv-a", options).FROM).toBe("/tmp/t3-direnv-a");
    expect(loadDirenvExportedEnv("/tmp/t3-direnv-b", options).FROM).toBe("/tmp/t3-direnv-b");
    expect(exports).toBe(2);
  });
});

describe("mergeDirenvExportedEnv", () => {
  it("leaves env unchanged when direnv has nothing to add", () => {
    const env = { PATH: "/bin", KEEP: "yes" };
    expect(
      mergeDirenvExportedEnv("/tmp/empty", env, {
        findEnvrc: () => undefined,
      }),
    ).toBe(env);
  });

  it("lets direnv overlay win on conflicts and preserves other keys", () => {
    const merged = mergeDirenvExportedEnv(
      "/tmp/t3-direnv-merge",
      { PATH: "/bin", OPENAI_API_KEY: "from-daemon", KEEP: "yes" },
      {
        findEnvrc: () => "/tmp/t3-direnv-merge/.envrc",
        resolveDirenvPath: () => "/usr/bin/direnv",
        runExport: () => ({
          status: 0,
          stdout: JSON.stringify({ OPENAI_API_KEY: "from-direnv", NEW: "added" }),
          stderr: "",
        }),
      },
    );
    expect(merged).toEqual({
      PATH: "/bin",
      OPENAI_API_KEY: "from-direnv",
      KEEP: "yes",
      NEW: "added",
    });
  });
});
