import { Button, Card, EmptyState } from '@gojona/ui';
import { Copy, Laptop, Layers, Terminal } from 'lucide-react';
import { STRATA_PROTOCOL } from '../types.ts';

/** Props accepted by `ZeroContainerCard`. */
export interface ZeroContainerCardProps {
  daemonOnline: boolean;
  onOpenSettings: () => void;
  onCopyText: (text: string, label: string) => Promise<void>;
}

/** Friendly onboarding card shown in KernelSU WebUI when zero containers are configured yet. */
export function ZeroContainerCard({
  daemonOnline,
  onOpenSettings,
  onCopyText,
}: ZeroContainerCardProps) {
  const createCmd = `strata rpc container.create '{"id":"${STRATA_PROTOCOL.DEFAULT_CONTAINER_ID}","distroId":"debian-12","sizeMb":4096,"setDefault":true}'`;

  if (!daemonOnline) {
    return (
      <Card className="p-5">
        <EmptyState
          icon={<Layers className="size-8" />}
          title="后台服务暂未运行"
          description="请先点击上方「启动后台服务」，服务就绪后即可查看容器或配置电脑端配对。"
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
    <Card className="flex flex-col gap-3 p-5">
      <EmptyState
        icon={<Layers className="size-8" />}
        title="尚未创建容器"
        description="当前设备还没有可用的 Linux 容器。你可以前往「系统设置」开启局域网配对，通过电脑端 Strata Desktop 创建或导入发行版镜像；也可以在 Root 终端执行下方命令创建默认容器。"
        action={
          <div className="flex flex-wrap items-center justify-center gap-2">
            <Button size="sm" variant="primary" onClick={onOpenSettings}>
              <Laptop className="size-3.5" />
              前往系统设置开启 PC 配对
            </Button>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void onCopyText(createCmd, '创建容器命令')}
            >
              <Copy className="size-3.5" />
              复制创建命令
            </Button>
          </div>
        }
      />

      <div className="flex items-center justify-between gap-2 rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <Terminal className="size-3.5 shrink-0 text-[var(--accent)]" />
          <code className="truncate font-mono text-xs text-[var(--text-primary)]">{createCmd}</code>
        </div>
        <Button
          className="shrink-0"
          size="sm"
          variant="ghost"
          aria-label="复制创建容器命令"
          onClick={() => void onCopyText(createCmd, '创建容器命令')}
        >
          <Copy className="size-3" />
          复制
        </Button>
      </div>
    </Card>
  );
}
