import { Button, StatusBanner } from '@gojona/ui';
import { Play } from 'lucide-react';
import type { ReactNode } from 'react';
import type { ContainerStatusSnapshot } from '../types/protocol.ts';

export interface ContainerScopedHeaderProps {
  title: string;
  subtitle: string;
  containers: ContainerStatusSnapshot[];
  activeContainer: ContainerStatusSnapshot | null;
  onSelectContainer: (id: string | undefined) => void;
  stoppedDescription: string;
  onStartContainer?: (containerId: string) => Promise<void>;
  extraActions?: ReactNode;
}

/**
 * Shared header bar with global target-container switcher and stopped-container warning banner.
 */
export function ContainerScopedHeader({
  title,
  subtitle,
  containers,
  activeContainer,
  onSelectContainer,
  stoppedDescription,
  onStartContainer,
  extraActions,
}: ContainerScopedHeaderProps) {
  return (
    <>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">{title}</h1>
          <p className="text-xs text-[var(--text-secondary)]">{subtitle}</p>
        </div>
        {containers.length > 0 && (
          <div className="flex flex-wrap items-center gap-2">
            <div className="flex items-center gap-1 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-1">
              {containers.map((c) => (
                <Button
                  key={c.id}
                  size="sm"
                  variant={activeContainer?.id === c.id ? 'pill' : 'ghost'}
                  onClick={() => onSelectContainer(c.id)}
                >
                  {c.id}
                </Button>
              ))}
            </div>
            {extraActions}
          </div>
        )}
      </div>

      {activeContainer && activeContainer.phase !== 'running' && (
        <StatusBanner
          tone="warning"
          title={`当前容器 ${activeContainer.id} 未运行`}
          description={stoppedDescription}
          action={
            onStartContainer && (
              <Button
                size="sm"
                variant="primary"
                onClick={() => void onStartContainer(activeContainer.id)}
              >
                <Play className="size-3.5" />
                一键启动容器
              </Button>
            )
          }
        />
      )}
    </>
  );
}
