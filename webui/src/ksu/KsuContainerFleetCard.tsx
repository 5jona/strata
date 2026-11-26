import { Badge, Button, Card, Input } from '@gojona/ui';
import { Check, Copy, HardDrive, Plus, Star, Terminal } from 'lucide-react';
import { useEffect, useId, useMemo, useState } from 'react';
import {
  buildBootstrapCommand,
  formatBytes,
  PHASE_META,
  parseSshPort,
  suggestNextContainerId,
  suggestNextSshPort,
  validateFleetCreateInput,
} from '../shared/strataFormat.ts';
import type { ContainerStatusSnapshot } from '../types.ts';
import type { CreateContainerOptions } from './useKsuActions.ts';

/** Props accepted by `KsuContainerFleetCard`. */
export interface KsuContainerFleetCardProps {
  containers: readonly ContainerStatusSnapshot[];
  selectedId: string | null;
  busyKey: string | null;
  onSelectContainer: (containerId: string) => void;
  onSetDefault: (containerId: string) => Promise<void>;
  onCreateContainer: (containerId: string, options?: CreateContainerOptions) => Promise<void>;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Adaptive multi-container fleet grid and quick custom-container creation form. */
export function KsuContainerFleetCard({
  containers,
  selectedId,
  busyKey,
  onSelectContainer,
  onSetDefault,
  onCreateContainer,
  onCopyText,
}: KsuContainerFleetCardProps) {
  const nameId = useId();
  const portId = useId();
  const sizeId = useId();

  const suggestedName = useMemo(() => suggestNextContainerId(containers), [containers]);
  const suggestedPort = useMemo(() => suggestNextSshPort(containers), [containers]);

  const [containerName, setContainerName] = useState(suggestedName);
  const [sshPortInput, setSshPortInput] = useState(String(suggestedPort));
  const [sizeGbInput, setSizeGbInput] = useState('8');

  const existingIds = useMemo(() => new Set(containers.map((c) => c.id)), [containers]);
  const occupiedPorts = useMemo(
    () => new Set(containers.map((c) => c.sshPort).filter((p) => p > 0)),
    [containers],
  );

  useEffect(() => {
    setContainerName((prev) => (existingIds.has(prev.trim()) ? suggestedName : prev));
    setSshPortInput((prev) => {
      const parsed = parseSshPort(prev);
      return parsed !== null && occupiedPorts.has(parsed) ? String(suggestedPort) : prev;
    });
  }, [existingIds, occupiedPorts, suggestedName, suggestedPort]);

  const busy = busyKey !== null;
  const { effectiveId, parsedPort, parsedSizeGb, idError, portError, sizeError } =
    validateFleetCreateInput(containerName, sshPortInput, sizeGbInput, existingIds, occupiedPorts);
  const canCreate = !busy && idError === null && portError === null && sizeError === null;
  const runningCount = containers.filter((c) => c.phase === 'running').length;
  const bootstrapCmd = buildBootstrapCommand(
    effectiveId || suggestedName,
    parsedPort ?? suggestedPort,
  );
  const handleCreate = async () => {
    if (!canCreate || parsedPort === null || parsedSizeGb === null) return;
    await onCreateContainer(effectiveId, { sshPort: parsedPort, imageSizeGb: parsedSizeGb });
  };

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-[var(--text-primary)]">容器实例与新建</span>
          <Badge variant="neutral">
            {containers.length} 个实例 · {runningCount} 运行中
          </Badge>
        </div>
        <span className="text-[11px] text-[var(--text-secondary)]">
          点击实例切换当前上下文，或直接创建新容器
        </span>
      </div>

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-12">
        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:col-span-7">
          {containers.map((c) => {
            const isSelected = c.id === selectedId;
            const isRunning = c.phase === 'running';
            const phase = PHASE_META[c.phase];
            return (
              <div
                key={c.id}
                className={`flex flex-col justify-between gap-2.5 rounded-[var(--radius-control)] border p-3 transition-colors ${
                  isSelected
                    ? 'border-[var(--accent)] bg-[var(--accent-soft)]/35'
                    : 'border-[var(--border-hairline)] bg-[var(--surface-elevated)]'
                }`}
              >
                <div className="flex flex-col gap-1.5">
                  <div className="flex flex-wrap items-center justify-between gap-1.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="truncate font-mono text-xs font-semibold text-[var(--text-primary)]">
                        {c.id}
                      </span>
                      {c.isDefault ? <Badge variant="accent">默认</Badge> : null}
                    </div>
                    <Badge variant={phase.variant} dot pulse={isRunning}>
                      {phase.label}
                    </Badge>
                  </div>

                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[11px] text-[var(--text-secondary)]">
                    <span>{c.sshEnabled ? `SSH :${c.sshPort}` : 'SSH 未启用'}</span>
                    <span className="inline-flex items-center gap-1">
                      <HardDrive className="size-3" />
                      {formatBytes(c.disk.imageAllocatedBytes)} /{' '}
                      {formatBytes(c.disk.imageVirtualBytes)}
                    </span>
                  </div>
                </div>

                <div className="flex flex-wrap items-center gap-1.5">
                  <Button
                    size="sm"
                    variant={isSelected ? 'pill' : 'secondary'}
                    disabled={isSelected}
                    onClick={() => onSelectContainer(c.id)}
                  >
                    {isSelected ? <Check className="size-3" /> : null}
                    {isSelected ? '当前选中' : '切换到此容器'}
                  </Button>
                  {!c.isDefault ? (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => void onSetDefault(c.id)}
                    >
                      <Star className="size-3" />
                      设为默认
                    </Button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </div>

        <div className="flex flex-col justify-between gap-3 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3.5 lg:col-span-5">
          <div className="flex flex-col gap-2.5">
            <span className="text-xs font-medium text-[var(--text-primary)]">新建自定义容器</span>

            <div className="flex flex-col gap-1">
              <label htmlFor={nameId} className="text-[11px] text-[var(--text-secondary)]">
                容器名称
              </label>
              <Input
                id={nameId}
                mono
                value={containerName}
                placeholder={suggestedName}
                aria-invalid={idError !== null}
                onChange={(e) => setContainerName(e.target.value)}
              />
              {idError ? (
                <p role="alert" className="text-[11px] text-[var(--danger)]">
                  {idError}
                </p>
              ) : null}
            </div>

            <div className="grid grid-cols-2 gap-2">
              <div className="flex flex-col gap-1">
                <label htmlFor={portId} className="text-[11px] text-[var(--text-secondary)]">
                  SSH 端口
                </label>
                <Input
                  id={portId}
                  mono
                  inputMode="numeric"
                  value={sshPortInput}
                  placeholder={String(suggestedPort)}
                  aria-invalid={portError !== null}
                  onChange={(e) => setSshPortInput(e.target.value)}
                />
              </div>
              <div className="flex flex-col gap-1">
                <label htmlFor={sizeId} className="text-[11px] text-[var(--text-secondary)]">
                  磁盘上限 (GB)
                </label>
                <Input
                  id={sizeId}
                  mono
                  inputMode="numeric"
                  value={sizeGbInput}
                  placeholder="8"
                  aria-invalid={sizeError !== null}
                  onChange={(e) => setSizeGbInput(e.target.value)}
                />
              </div>
            </div>

            {portError || sizeError ? (
              <p role="alert" className="text-[11px] text-[var(--danger)]">
                {portError ?? sizeError}
              </p>
            ) : null}
          </div>

          <div className="flex flex-col gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                size="sm"
                variant="primary"
                disabled={!canCreate}
                onClick={() => void handleCreate()}
              >
                <Plus className="size-3.5" />
                创建容器
              </Button>
              <Button
                size="sm"
                variant="secondary"
                disabled={!canCreate}
                onClick={() => void onCopyText(bootstrapCmd, '初始化脚本命令')}
              >
                <Copy className="size-3.5" />
                复制初始化命令
              </Button>
            </div>
            <div className="flex min-w-0 items-center gap-1.5 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-base)] px-2.5 py-1.5">
              <Terminal className="size-3 shrink-0 text-[var(--accent)]" />
              <code className="truncate font-mono text-[11px] text-[var(--text-secondary)]">
                {bootstrapCmd}
              </code>
            </div>
          </div>
        </div>
      </div>
    </Card>
  );
}
