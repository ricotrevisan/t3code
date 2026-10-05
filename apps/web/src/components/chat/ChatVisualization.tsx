import type { ScopedThreadRef } from "@t3tools/contracts";
import { useState, type ReactNode } from "react";
import { useAssetUrlRefresh, useAssetUrlState } from "../../assets/assetUrls";
import { BrowserDocumentFrame } from "../files/BrowserDocumentFrame";

/** Uses the originating environment's signed file URL, including for remote clients. */
export function ChatVisualization(props: {
  readonly threadRef: ScopedThreadRef;
  readonly path: string;
  readonly title: string;
  readonly fallback: ReactNode;
}) {
  const resource = {
    _tag: "media-file" as const,
    threadId: props.threadRef.threadId,
    path: props.path,
  };
  const asset = useAssetUrlState(props.threadRef.environmentId, resource);
  const refresh = useAssetUrlRefresh(props.threadRef.environmentId, resource);
  const [retryError, setRetryError] = useState(false);
  return (
    <section
      className="my-3 overflow-hidden rounded-lg border border-border"
      aria-label={props.title}
    >
      <div className="flex items-center justify-between gap-3 border-b border-border px-3 py-2 text-sm">
        <span>{props.title}</span>
        <button
          type="button"
          onClick={() => {
            setRetryError(false);
            void refresh().catch(() => setRetryError(true));
          }}
        >
          Reload
        </button>
      </div>
      {asset._tag === "Success" ? (
        <div className="flex h-[640px] max-h-[80vh] flex-col">
          <BrowserDocumentFrame src={asset.url} title={props.title} pdf={false} />
        </div>
      ) : (
        <div className="p-4 text-sm text-muted-foreground" role="status">
          {asset._tag === "Failure" || retryError ? (
            <>Unable to load the visualization. {props.fallback}</>
          ) : (
            "Loading visualization…"
          )}
        </div>
      )}
    </section>
  );
}
