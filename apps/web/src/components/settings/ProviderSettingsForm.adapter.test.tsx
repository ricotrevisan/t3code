import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vite-plus/test";

import { AdapterConfigForm } from "./ProviderSettingsForm";

describe("AdapterConfigForm", () => {
  it("renders supported and unsupported fields without treating password format as secret", () => {
    const markup = renderToStaticMarkup(
      createElement(AdapterConfigForm, {
        schema: {
          type: "object",
          required: ["token", "nested"],
          properties: {
            token: {
              type: "string",
              title: "Access token",
              description: "Use the environment editor for sensitive values.",
              format: "password",
            },
            nested: { type: "object", properties: { value: { type: "string" } } },
          },
        },
        value: {},
        idPrefix: "adapter",
        variant: "dialog",
        onChange: vi.fn(),
      }),
    );

    expect(markup).toContain("Access token");
    expect(markup).toContain("Access token is required.");
    expect(markup).toContain("Nested");
    expect(markup).toContain("cannot edit");
    expect(markup).toContain("Nested is required but cannot be edited by this client.");
    expect(markup).not.toContain('type="password"');
  });

  it("renders the blocking error for an unsupported root without a default", () => {
    const markup = renderToStaticMarkup(
      createElement(AdapterConfigForm, {
        schema: { type: "array", items: { type: "string" } },
        value: undefined,
        idPrefix: "adapter-root",
        variant: "dialog",
        onChange: vi.fn(),
      }),
    );

    expect(markup).toContain("required configuration cannot be edited by this client");
  });
});
