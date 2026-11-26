import { Badge, Button, Card, Input, SettingRow, Slider, StatusBanner, Switch } from '@gojona/ui';
import { ChevronDown, ChevronUp } from 'lucide-react';
import { useState } from 'react';
import type { InstallSourceMode } from './installHelpers.ts';

export interface InstallFormValues {
  containerId: string;
  hostname: string;
  hostnameLinked: boolean;
  sizeGb: number;
  enableSsh: boolean;
  sshPort: string;
  sshPubKeysRaw: string;
  autoStart: boolean;
  setDefault: boolean;
  hostBridge: boolean;
  noatime: boolean;
  l5DiagnosticMounts: boolean;
  syncAndroidDns: boolean;
  extraCharDevs: string[];
}

export interface InstallConfigFormProps {
  sourceMode: InstallSourceMode;
  isRawImg: boolean;
  values: InstallFormValues;
  idError: string | null;
  sshPortError: string | null;
  urlOrShaError: string | null;
  onChange: (patch: Partial<InstallFormValues>) => void;
  submitAction: React.ReactNode;
}

/** Decoupled container configuration form with explicit SSH/bridge/default toggles & advanced accordion. */
export function InstallConfigForm({
  sourceMode,
  isRawImg,
  values,
  idError,
  sshPortError,
  urlOrShaError,
  onChange,
  submitAction,
}: InstallConfigFormProps) {
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const toggleDev = (dev: string, checked: boolean) => {
    const next = checked
      ? Array.from(new Set([...values.extraCharDevs, dev]))
      : values.extraCharDevs.filter((d) => d !== dev);
    onChange({ extraCharDevs: next });
  };

  return (
    <Card className="flex flex-col gap-3 p-4">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
        <div className="flex flex-col gap-1">
          <span className="text-xs text-[var(--text-secondary)]">容器名称 (ID)</span>
          <Input
            mono
            value={values.containerId}
            onChange={(e) => {
              const nextId = e.target.value;
              onChange({
                containerId: nextId,
                ...(values.hostnameLinked ? { hostname: nextId } : {}),
              });
            }}
          />
          {idError && <span className="text-[11px] text-[var(--danger)]">{idError}</span>}
        </div>

        <div className="flex flex-col gap-1">
          <div className="flex items-center justify-between">
            <span className="text-xs text-[var(--text-secondary)]">容器主机名 (hostname)</span>
            <button
              type="button"
              className="text-[11px] text-[var(--accent)] hover:underline"
              onClick={() =>
                onChange({
                  hostnameLinked: !values.hostnameLinked,
                  ...(!values.hostnameLinked ? { hostname: values.containerId } : {}),
                })
              }
            >
              {values.hostnameLinked ? '解绑独立编辑' : '跟随容器 ID'}
            </button>
          </div>
          <Input
            mono
            disabled={values.hostnameLinked}
            value={values.hostname}
            onChange={(e) => onChange({ hostname: e.target.value })}
          />
        </div>

        {sourceMode !== 'clone' && (
          <div className="flex items-end">
            <Slider
              className="w-full"
              label={isRawImg ? '最大解压保护上限' : '稀疏镜像虚拟容量上限'}
              min={8}
              max={512}
              step={4}
              unit=" GiB"
              presets={[
                { label: '16 GiB', value: 16 },
                { label: '32 GiB', value: 32 },
                { label: '64 GiB', value: 64 },
                { label: '128 GiB', value: 128 },
              ]}
              value={values.sizeGb}
              onValueChange={(sizeGb) => onChange({ sizeGb })}
            />
          </div>
        )}
      </div>

      {isRawImg && sourceMode !== 'clone' && (
        <StatusBanner
          tone="info"
          title="原始磁盘镜像 (.img / .img.gz) 容量说明"
          description="上方滑块仅作为解压体积安全保护上限，不会对原镜像强行缩容或重新格式化。"
        />
      )}

      {/* Explicit decoupled toggles */}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {sourceMode !== 'clone' && (
          <SettingRow
            variant="plain"
            title="启用 Dropbear SSH"
            description="关闭即可创建无 SSH 纯净容器"
            control={
              <Switch
                checked={values.enableSsh}
                onCheckedChange={(enableSsh) => onChange({ enableSsh })}
              />
            }
          />
        )}
        <SettingRow
          variant="plain"
          title="开机自启"
          control={
            <Switch
              checked={values.autoStart}
              onCheckedChange={(autoStart) => onChange({ autoStart })}
            />
          }
        />
        {sourceMode !== 'clone' && (
          <SettingRow
            variant="plain"
            title="宿主机命令桥接"
            description="注入 host-exec 与 strata-debug"
            control={
              <Switch
                checked={values.hostBridge}
                onCheckedChange={(hostBridge) => onChange({ hostBridge })}
              />
            }
          />
        )}
        <SettingRow
          variant="plain"
          title="设为默认调试容器"
          control={
            <Switch
              checked={values.setDefault}
              onCheckedChange={(setDefault) => onChange({ setDefault })}
            />
          }
        />
      </div>

      {/* SSH section shown only when enabled */}
      {(values.enableSsh || sourceMode === 'clone') && (
        <div className="flex flex-col gap-2.5 rounded-xl border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3">
          {sourceMode !== 'clone' && (
            <StatusBanner
              tone="warning"
              title="启用 SSH 需要目标镜像内含 dropbear-bin（Debian/Ubuntu 可在创建后补装）"
              description="若使用 Alpine 或最小根文件系统且未安装 dropbear，容器可能无法通过 SSH 登录，仍可使用内置 WebSocket 终端。"
            />
          )}
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <div className="flex flex-col gap-1">
              <span className="text-xs text-[var(--text-secondary)]">Dropbear SSH 端口</span>
              <Input
                mono
                value={values.sshPort}
                onChange={(e) => onChange({ sshPort: e.target.value })}
              />
              {sshPortError && (
                <span className="text-[11px] text-[var(--danger)]">{sshPortError}</span>
              )}
            </div>
            {sourceMode !== 'clone' && (
              <div className="flex flex-col gap-1 sm:col-span-2">
                <span className="text-xs text-[var(--text-secondary)]">
                  预注入 SSH 公钥（支持多行，每行一条）
                </span>
                <textarea
                  rows={2}
                  value={values.sshPubKeysRaw}
                  placeholder="ssh-ed25519 AAAA... user@pc"
                  onChange={(e) => onChange({ sshPubKeysRaw: e.target.value })}
                  className="w-full rounded-[var(--radius-control)] border border-[var(--border-hairline)] bg-[var(--surface-card)] p-2 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                />
              </div>
            )}
          </div>
        </div>
      )}

      {/* Advanced options */}
      {sourceMode !== 'clone' && (
        <div className="border-t border-[var(--border-hairline)] pt-2">
          <Button size="sm" variant="ghost" onClick={() => setAdvancedOpen((v) => !v)}>
            {advancedOpen ? (
              <ChevronUp className="size-3.5" />
            ) : (
              <ChevronDown className="size-3.5" />
            )}
            高级挂载、网络与调试透传选项
            <Badge variant="neutral">可选</Badge>
          </Button>
          {advancedOpen && (
            <div className="mt-2 grid grid-cols-1 gap-2 rounded-xl border border-[var(--border-hairline)] bg-[var(--surface-elevated)] p-3 sm:grid-cols-2">
              <SettingRow
                variant="plain"
                title="noatime 挂载"
                description="减少闪存写入放大"
                control={
                  <Switch
                    checked={values.noatime}
                    onCheckedChange={(noatime) => onChange({ noatime })}
                  />
                }
              />
              <SettingRow
                variant="plain"
                title="同步 Android 系统 DNS"
                control={
                  <Switch
                    checked={values.syncAndroidDns}
                    onCheckedChange={(syncAndroidDns) => onChange({ syncAndroidDns })}
                  />
                }
              />
              <SettingRow
                variant="plain"
                title="L5 内核诊断只读挂载"
                description="挂载 pstore / tracing / bpf"
                control={
                  <Switch
                    checked={values.l5DiagnosticMounts}
                    onCheckedChange={(l5DiagnosticMounts) => onChange({ l5DiagnosticMounts })}
                  />
                }
              />
              <div className="flex flex-wrap items-center gap-3 px-1">
                <span className="text-xs text-[var(--text-secondary)]">字符设备透传：</span>
                {['uinput', 'net/tun'].map((dev) => (
                  <div
                    key={dev}
                    className="flex items-center gap-1.5 font-mono text-xs text-[var(--text-primary)]"
                  >
                    <Switch
                      checked={values.extraCharDevs.includes(dev)}
                      onCheckedChange={(checked) => toggleDev(dev, checked)}
                    />
                    <span>{dev}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}

      {urlOrShaError && <StatusBanner tone="danger" title={urlOrShaError} />}

      <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
        <span className="text-xs text-[var(--text-secondary)]">
          自动应用 Android GKI 兼容设置（APT 根用户沙盒配置与隔离命名空间）
        </span>
        {submitAction}
      </div>
    </Card>
  );
}
