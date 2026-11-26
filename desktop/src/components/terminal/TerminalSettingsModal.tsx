import { Button, Card, cn, SegmentedControl, SettingRow, Slider, Switch } from '@gojona/ui';
import { RotateCcw, SlidersHorizontal, X } from 'lucide-react';
import { useEffect } from 'react';
import {
  DEFAULT_TERMINAL_PREFS,
  MAX_FONT_SIZE,
  MIN_FONT_SIZE,
  TERMINAL_SHELL_OPTIONS,
  type TerminalCursorStyle,
  type TerminalPrefs,
  type TerminalRendererPref,
  type TerminalShellId,
} from './terminalPrefs.ts';
import { TERMINAL_THEME_OPTIONS, type TerminalThemeId } from './terminalThemes.ts';

export interface TerminalSettingsModalProps {
  open: boolean;
  prefs: TerminalPrefs;
  onUpdate: (patch: Partial<TerminalPrefs>) => void;
  onClose: () => void;
}

const LINE_HEIGHT_OPTIONS = [1.0, 1.1, 1.2, 1.3, 1.4, 1.5];

/** Modal dialog configuring terminal theme, typography, cursor, renderer, and shell defaults. */
export function TerminalSettingsModal({
  open,
  prefs,
  onUpdate,
  onClose,
}: TerminalSettingsModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="终端外观与交互设置"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/45 p-4 backdrop-blur-[2px]"
      onMouseDown={(ev) => {
        if (ev.target === ev.currentTarget) onClose();
      }}
    >
      <Card className="flex max-h-[86vh] w-full max-w-lg flex-col overflow-hidden shadow-2xl">
        <div className="flex items-center justify-between border-b border-[var(--border-hairline)] px-4 py-3">
          <div className="flex items-center gap-2">
            <SlidersHorizontal className="size-4 text-[var(--accent)]" />
            <h2 className="text-sm font-semibold text-[var(--text-primary)]">终端外观与交互设置</h2>
          </div>
          <div className="flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onUpdate({ ...DEFAULT_TERMINAL_PREFS })}
            >
              <RotateCcw className="size-3.5" />
              恢复默认
            </Button>
            <Button size="icon-sm" variant="ghost" aria-label="关闭设置" onClick={onClose}>
              <X className="size-4" />
            </Button>
          </div>
        </div>

        <div className="flex flex-col gap-4 overflow-y-auto p-4 strata-scroll">
          {/* Theme selector */}
          <div className="space-y-2">
            <div className="text-xs font-medium text-[var(--text-secondary)]">
              配色方案（16 色 ANSI）
            </div>
            <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
              {TERMINAL_THEME_OPTIONS.map((opt) => {
                const selected = prefs.themeId === opt.id;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    onClick={() => onUpdate({ themeId: opt.id as TerminalThemeId })}
                    className={cn(
                      'flex items-center justify-between rounded-lg border px-3 py-2 text-left transition-colors',
                      selected
                        ? 'border-[var(--accent)] bg-[var(--accent)]/12'
                        : 'border-[var(--border-hairline)] bg-[var(--surface-elevated)] hover:border-[var(--border-subtle)]',
                    )}
                  >
                    <div className="min-w-0 pr-2">
                      <div className="truncate text-xs font-medium text-[var(--text-primary)]">
                        {opt.label}
                      </div>
                      <div className="truncate text-[10px] text-[var(--text-secondary)]">
                        {opt.description}
                      </div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {opt.swatches.map((hex) => (
                        <span
                          key={`${opt.id}-${hex}`}
                          className="size-3 rounded-full border border-black/20"
                          style={{ backgroundColor: hex }}
                        />
                      ))}
                    </div>
                  </button>
                );
              })}
            </div>
          </div>

          {/* Font size & line height */}
          <div className="space-y-2 border-t border-[var(--border-hairline)] pt-3">
            <SettingRow
              variant="plain"
              title={`字号 (${prefs.fontSize}px)`}
              description="内置 Maple Mono NF CN · 支持 Ctrl+滚轮 或 Ctrl+± 实时缩放"
              control={
                <div className="w-40">
                  <Slider
                    min={MIN_FONT_SIZE}
                    max={MAX_FONT_SIZE}
                    step={1}
                    value={prefs.fontSize}
                    onValueChange={(val) => onUpdate({ fontSize: val })}
                  />
                </div>
              }
            />

            <SettingRow
              variant="plain"
              title="行高"
              description="控制行间距与中英文混排舒适度"
              control={
                <div className="flex items-center gap-1">
                  {LINE_HEIGHT_OPTIONS.map((lh) => (
                    <button
                      key={lh}
                      type="button"
                      onClick={() => onUpdate({ lineHeight: lh })}
                      className={cn(
                        'rounded px-2 py-1 font-mono text-[11px] transition-colors',
                        prefs.lineHeight === lh
                          ? 'bg-[var(--accent)] text-white'
                          : 'bg-[var(--surface-elevated)] text-[var(--text-secondary)] hover:text-[var(--text-primary)]',
                      )}
                    >
                      {lh.toFixed(1)}
                    </button>
                  ))}
                </div>
              }
            />
          </div>

          {/* Cursor & renderer */}
          <div className="space-y-2 border-t border-[var(--border-hairline)] pt-3">
            <SettingRow
              variant="plain"
              title="光标样式"
              description="实时生效于所有打开的终端窗格"
              control={
                <SegmentedControl<TerminalCursorStyle>
                  size="sm"
                  value={prefs.cursorStyle}
                  onValueChange={(val) => onUpdate({ cursorStyle: val })}
                  options={[
                    { value: 'block', label: '方块' },
                    { value: 'bar', label: '竖线' },
                    { value: 'underline', label: '下划线' },
                  ]}
                />
              }
            />

            <SettingRow
              variant="plain"
              title="光标闪烁"
              description="关闭可减少静止时的视觉干扰"
              control={
                <Switch
                  checked={prefs.cursorBlink}
                  onCheckedChange={(checked) => onUpdate({ cursorBlink: checked })}
                />
              }
            />

            <SettingRow
              variant="plain"
              title="渲染引擎"
              description="WebGL2 性能最佳；如遇驱动异常可切换为 DOM 兼容渲染"
              control={
                <SegmentedControl<TerminalRendererPref>
                  size="sm"
                  value={prefs.renderer}
                  onValueChange={(val) => onUpdate({ renderer: val })}
                  options={[
                    { value: 'webgl', label: 'WebGL2' },
                    { value: 'dom', label: 'DOM' },
                  ]}
                />
              }
            />
          </div>

          {/* Interaction & default shell */}
          <div className="space-y-2 border-t border-[var(--border-hairline)] pt-3">
            <SettingRow
              variant="plain"
              title="默认新建 Shell"
              description="新标签页或快捷分屏默认使用的交互式 Shell"
              control={
                <select
                  value={prefs.defaultShell}
                  onChange={(ev) => onUpdate({ defaultShell: ev.target.value as TerminalShellId })}
                  className="rounded-md border border-[var(--border-hairline)] bg-[var(--surface-elevated)] px-2.5 py-1 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                >
                  {TERMINAL_SHELL_OPTIONS.map((opt) => (
                    <option key={opt.id} value={opt.id}>
                      {opt.label} — {opt.hint}
                    </option>
                  ))}
                </select>
              }
            />

            <SettingRow
              variant="plain"
              title="选中自动复制"
              description="鼠标拖选终端文字后立即写入系统剪贴板"
              control={
                <Switch
                  checked={prefs.copyOnSelect}
                  onCheckedChange={(checked) => onUpdate({ copyOnSelect: checked })}
                />
              }
            />

            <SettingRow
              variant="plain"
              title="右键直接粘贴"
              description="开启后右键直接安全粘贴，按住 Shift + 右键仍可打开菜单"
              control={
                <Switch
                  checked={prefs.rightClickPaste}
                  onCheckedChange={(checked) => onUpdate({ rightClickPaste: checked })}
                />
              }
            />
          </div>
        </div>
      </Card>
    </div>
  );
}
