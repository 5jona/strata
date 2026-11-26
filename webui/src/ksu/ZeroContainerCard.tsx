import { Button, Card, EmptyState, Input } from '@gojona/ui';
import { Copy, Laptop, Layers, Plus, Terminal } from 'lucide-react';
import { useId, useState } from 'react';
import { buildBootstrapCommand, isValidContainerId } from '../shared/strataFormat.ts';
import { STRATA_PROTOCOL } from '../types.ts';

/** Props accepted by `ZeroContainerCard`. */
export interface ZeroContainerCardProps {
  daemonOnline: boolean;
  busy?: boolean;
  onOpenSettings: () => void;
  onCopyText: (text: string, label: string) => Promise<void>;
  onCreateContainer?: (containerId: string) => Promise<void>;
}

/** Onboarding card shown in KernelSU WebUI when zero containers are configured yet. */
export function ZeroContainerCard({
  daemonOnline,
  busy = false,
  onOpenSettings,
  onCopyText,
  onCreateContainer,
}: ZeroContainerCardProps) {
  const inputId = useId();
  const [containerName, setContainerName] = useState<string>(STRATA_PROTOCOL.DEFAULT_CONTAINER_ID);
  const trimmedName = containerName.trim();
  const effectiveId = trimmedName || STRATA_PROTOCOL.DEFAULT_CONTAINER_ID;
  const isValidId = isValidContainerId(effectiveId);

  const createRpcCmd = `strata rpc container.create '{"id":"${effectiveId}","distroId":"debian-12","sizeMb":4096,"setDefault":true}'`;
  const bootstrapCmd = buildBootstrapCommand(effectiveId);

  if (!daemonOnline) {
    return (
      <Card className="p-5">
        <EmptyState
          icon={<Layers className="size-8" />}
          title="后台服务暂未运行"
          description="请先点击上方「启动后台服务」，服务就绪后即可创建容器或配置电脑端配对。"
          action={
            <Button size="sm" variant="secondary" onClick={onOpenSettings}>
              <Laptop className="size-3.5" />
              查看系统设置
            </Button>
          }
        />
      </Card>
    );
  }

  return (
    <Card className="flex flex-col gap-4 p-5">
      <EmptyState
        icon={<Layers className="size-8" />}
        title="尚未创建容器"
        description="当前设备还没有可用的 Linux 容器。你可以自定义初始容器名称后直接在设备端创建，或前往「系统设置」开启局域网配对连接 Strata Desktop，也可复制下方命令在终端执行。"
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            {onCreateContainer ? (
              <Button
                size="sm"
                variant="primary"
                disabled={busy || !isValidId}
                onClick={() => void onCreateContainer(effectiveId)}
              >
                <Plus className="size-3.5" />
                立即创建容器 ({effectiveId})
              </Button>
            ) : null}
            <Button size="sm" variant="secondary" onClick={onOpenSettings}>
              <Laptop className="size-3.5" />
              前往系统设置开启 PC 配对
            </Button>
          </div>
        }
      />

      <div className="flex flex-col gap-1.5 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3">
        <label htmlFor={inputId} className="text-xs font-medium text-[var(--text-primary)]">
          自定义初始容器名称
        </label>
        <div className="flex flex-col gap-1.5 sm:flex-row sm:items-center">
          <Input
            id={inputId}
            mono
            value={containerName}
            placeholder={STRATA_PROTOCOL.DEFAULT_CONTAINER_ID}
            aria-label="自定义初始容器名称"
            aria-invalid={!isValidId}
            onChange={(e) => setContainerName(e.target.value)}
          />
          <span className="shrink-0 text-[11px] text-[var(--text-secondary)]">
            支持字母、数字、- 与 _（最多 32 位）
          </span>
        </div>
        {!isValidId ? (
          <p role="alert" className="text-[11px] text-[var(--danger)]">
            容器名称需以字母或数字开头，仅包含字母、数字、短横线或下划线。
          </p>
        ) : null}
      </div>

      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        {[
          { label: '设备端创建命令', cmd: createRpcCmd },
          { label: '脚本初始化命令', cmd: bootstrapCmd },
        ].map(({ label, cmd }) => (
          <div
            key={label}
            className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2"
          >
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="text-[10px] text-[var(--text-secondary)]">{label}</span>
              <div className="flex min-w-0 items-center gap-1.5">
                <Terminal className="size-3.5 shrink-0 text-[var(--accent)]" />
                <code className="truncate font-mono text-xs text-[var(--text-primary)]">{cmd}</code>
              </div>
            </div>
            <Button
              className="shrink-0"
              size="sm"
              variant="ghost"
              aria-label={`复制${label}`}
              disabled={!isValidId}
              onClick={() => void onCopyText(cmd, label)}
            >
              <Copy className="size-3" />
              复制
            </Button>
          </div>
        ))}
      </div>
    </Card>
  );
}
