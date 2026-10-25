// @effect-diagnostics nodeBuiltinImport:off
import * as NodeURL from "node:url";
import * as NodeServices from "@effect/platform-node/NodeServices";
import { it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import { describe, expect, vi } from "vite-plus/test";
import { preparePrimeUserInputExtension } from "./primeUserInputExtension.ts";

type Input = { question: string; options?: string[] };
type UI = {
  select: (
    title: string,
    options: string[],
    opts: { signal?: AbortSignal },
  ) => Promise<string | undefined>;
  input: (
    title: string,
    placeholder: undefined,
    opts: { signal?: AbortSignal },
  ) => Promise<string | undefined>;
};
interface Tool {
  name: string;
  parameters: unknown;
  execute: (
    id: string,
    params: Input,
    signal: AbortSignal | undefined,
    update: undefined,
    ctx: { hasUI: boolean; ui: UI },
  ) => Promise<{
    content: { type: string; text: string }[];
    details: { cancelled: boolean; answer?: string };
  }>;
}

const withTool = (check: (tool: Tool) => Promise<void>) =>
  Effect.scoped(
    Effect.gen(function* () {
      const fs = yield* FileSystem.FileSystem;
      const dir = yield* fs.makeTempDirectoryScoped({ prefix: "t3-prime-user-input-" });
      const path = yield* preparePrimeUserInputExtension(dir);
      expect(yield* preparePrimeUserInputExtension(dir)).toBe(path);
      const loaded = yield* Effect.promise(() => import(NodeURL.pathToFileURL(path).href));
      let tool: Tool | undefined;
      const factory = loaded.default as (api: { registerTool: (value: Tool) => void }) => void;
      factory({
        registerTool: (value) => {
          tool = value;
        },
      });
      const registeredTool = tool;
      if (!registeredTool) return yield* Effect.die(new Error("Extension did not register a tool"));
      yield* Effect.promise(() => check(registeredTool));
    }),
  ).pipe(Effect.provide(NodeServices.layer));

describe("primeUserInputExtension", () => {
  it.live("registers a tool that asks a single choice with the turn signal", () =>
    withTool(async (tool) => {
      const signal = new AbortController().signal;
      const ui = { select: vi.fn(async () => "Blue"), input: vi.fn(async () => undefined) };
      expect(tool.name).toBe("request_user_input");
      const result = await tool.execute(
        "call",
        { question: " Color? ", options: [" Red ", "Blue"] },
        signal,
        undefined,
        { hasUI: true, ui },
      );
      expect(ui.select).toHaveBeenCalledWith("Color?", ["Red", "Blue"], { signal });
      expect(ui.input).not.toHaveBeenCalled();
      expect(result.details).toEqual({ cancelled: false, answer: "Blue" });
      expect(JSON.parse(result.content[0]!.text)).toEqual(result.details);
    }),
  );

  it.live("preserves free text including an empty answer and reports cancellation", () =>
    withTool(async (tool) => {
      const signal = new AbortController().signal;
      const ui = { select: vi.fn(async () => undefined), input: vi.fn<UI["input"]>() };
      for (const answer of ["Some detail", "", undefined]) {
        ui.input.mockResolvedValueOnce(answer);
        const result = await tool.execute("call", { question: "Details?" }, signal, undefined, {
          hasUI: true,
          ui,
        });
        expect(result.details).toEqual(
          answer === undefined ? { cancelled: true } : { cancelled: false, answer },
        );
      }
      expect(ui.input).toHaveBeenCalledWith("Details?", undefined, { signal });
      expect(ui.select).not.toHaveBeenCalled();
      expect(
        (
          await tool.execute("call", { question: "Pick?", options: ["A"] }, signal, undefined, {
            hasUI: true,
            ui,
          })
        ).details,
      ).toEqual({ cancelled: true });
    }),
  );

  it.live("does not ask after abort and does not treat an abort as an answer", () =>
    withTool(async (tool) => {
      const controller = new AbortController();
      const ui = {
        select: vi.fn(async () => undefined),
        input: vi.fn(async () => {
          controller.abort();
          return "late";
        }),
      };
      const run = () =>
        tool.execute("call", { question: "Continue?" }, controller.signal, undefined, {
          hasUI: true,
          ui,
        });
      expect((await run()).details).toEqual({ cancelled: true });
      expect((await run()).details).toEqual({ cancelled: true });
      expect(ui.input).toHaveBeenCalledTimes(1);
    }),
  );

  it.live("rejects unavailable UI and ambiguous questions before prompting", () =>
    withTool(async (tool) => {
      const ui = { select: vi.fn(async () => undefined), input: vi.fn(async () => undefined) };
      await expect(
        tool.execute("call", { question: "Question" }, undefined, undefined, { hasUI: false, ui }),
      ).rejects.toThrow("unavailable");
      for (const params of [
        { question: " " },
        { question: "Q", options: [] },
        { question: "Q", options: [" "] },
        { question: "Q", options: ["A", " A "] },
      ]) {
        await expect(
          tool.execute("call", params, undefined, undefined, { hasUI: true, ui }),
        ).rejects.toThrow("nonempty");
      }
      expect(ui.input).not.toHaveBeenCalled();
      expect(ui.select).not.toHaveBeenCalled();
    }),
  );
});
