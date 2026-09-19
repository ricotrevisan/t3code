import { describe, expect, expectTypeOf, it } from "vite-plus/test";

import {
  ProviderAdapterHostResourceError,
  type ProviderAdapterHostV1,
  type ProviderAdapterHostV2,
  type ProviderAdapterPackageV1,
} from "./index.ts";

type DefaultPackageHost = Parameters<ProviderAdapterPackageV1["create"]>[1];
type V2PackageHost = Parameters<
  ProviderAdapterPackageV1<unknown, ProviderAdapterHostV2>["create"]
>[1];

describe("ProviderAdapterPackageV1", () => {
  it("can require either host protocol without widening the create environment", () => {
    expectTypeOf<DefaultPackageHost>().toEqualTypeOf<ProviderAdapterHostV1>();
    expectTypeOf<V2PackageHost>().toEqualTypeOf<ProviderAdapterHostV2>();
  });
});

describe("ProviderAdapterHostResourceError", () => {
  it("identifies the failed host resource operation", () => {
    const error = new ProviderAdapterHostResourceError({
      operation: "attachments.read",
      detail: "Attachment is unavailable",
    });

    expect(error._tag).toBe("ProviderAdapterHostResourceError");
    expect(error.message).toBe(
      "Host adapter resource failed in attachments.read: Attachment is unavailable",
    );
  });
});
