import { describe, expect, it, vi } from "vite-plus/test";

const iconMocks = vi.hoisted(() => ({
  IconCpu: vi.fn(() => null),
  Path: vi.fn(() => null),
  Svg: vi.fn(() => null),
}));

vi.mock("@tabler/icons-react-native", () => ({ IconCpu: iconMocks.IconCpu }));
vi.mock("react-native-svg", () => ({ Path: iconMocks.Path, Svg: iconMocks.Svg }));
vi.mock("../features/settings/appearance/AppearancePreferencesProvider", () => ({
  useAppearancePreferences: () => ({ themeAppearance: "light" }),
}));

import { ProviderIcon } from "./ProviderIcon";

describe("ProviderIcon", () => {
  it("keeps the Codex glyph limited to the codex driver", () => {
    expect(ProviderIcon({ provider: "codex" }).type).toBe(iconMocks.Svg);
    expect(ProviderIcon({ provider: "external-driver" }).type).toBe(iconMocks.IconCpu);
    expect(ProviderIcon({ provider: null }).type).toBe(iconMocks.IconCpu);
  });
});
