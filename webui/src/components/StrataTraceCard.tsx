import { type BridgeTraceEvent, useBridge } from '@gojona/bridge';
import { BridgeTraceInspector } from '@gojona/ui';
import { useCallback, useEffect, useState } from 'react';

/** Props accepted by `StrataTraceCard`. */
export interface StrataTraceCardProps {
  onCopyLog?: (text: string) => Promise<boolean> | boolean;
}

const MAX_TRACE_ENTRIES = 100;

/** Live inspector displaying recent SideVeil bridge RPC and event trace entries. */
export function StrataTraceCard({ onCopyLog }: StrataTraceCardProps) {
  const bridge = useBridge();
  const [entries, setEntries] = useState<BridgeTraceEvent[]>(() =>
    bridge.getTraceEntries(MAX_TRACE_ENTRIES),
  );

  useEffect(() => {
    setEntries(bridge.getTraceEntries(MAX_TRACE_ENTRIES));
    return bridge.subscribeTrace(() => {
      setEntries(bridge.getTraceEntries(MAX_TRACE_ENTRIES));
    });
  }, [bridge]);

  const handleClear = useCallback(() => {
    bridge.clearTraceEntries();
    setEntries([]);
  }, [bridge]);

  return (
    <BridgeTraceInspector
      title="Bridge 通信追踪"
      entries={entries}
      onClear={handleClear}
      onCopy={onCopyLog}
      maxHeightClassName="max-h-56"
    />
  );
}
