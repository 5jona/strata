import type { BindMount } from './generated/configTypes.ts';
import { L5_DIAG_PATHS } from './generated/constants.ts';

export * from './generated/configTypes.ts';
export * from './generated/constants.ts';
export * from './generated/rpc.ts';
export * from './generated/runtimeTypes.ts';

/** Canonical opt-in read-only L5 kernel diagnostic bind mounts. */
export const L5_DIAG_MOUNTS: readonly BindMount[] = [
  { hostPath: L5_DIAG_PATHS.PSTORE, containerPath: L5_DIAG_PATHS.PSTORE, readOnly: true },
  { hostPath: L5_DIAG_PATHS.TRACING, containerPath: L5_DIAG_PATHS.TRACING, readOnly: true },
  { hostPath: L5_DIAG_PATHS.BPF, containerPath: L5_DIAG_PATHS.BPF, readOnly: true },
];
