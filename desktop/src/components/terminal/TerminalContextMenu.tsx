import { cn } from '@gojona/ui';
import {
  ClipboardCopy,
  ClipboardPaste,
  Columns2,
  Eraser,
  ExternalLink,
  RotateCcw,
  Rows2,
  Search,
} from 'lucide-react';
import { useEffect, useRef } from 'react';
import { isMacPlatform } from './terminalPlatform.ts';

export interface TerminalContextMenuState {
  x: number;
  y: number;
  hasSelection: boolean;
}

export interface TerminalContextMenuProps {
  menu: TerminalContextMenuState | null;
  canSplit: boolean;
  onClose: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onClear: () => void;
  onSearch: () => void;
  onSplitRight: () => void;
  onSplitDown: () => void;
  onReset: () => void;
  onOpenExternalSsh?: () => void;
}

interface MenuItemDef {
  id: string;
  label: string;
  shortcut?: string;
  icon: typeof ClipboardCopy;
  disabled?: boolean;
  dividerBefore?: boolean;
  onClick: () => void;
}

/** Floating right-click context menu for xterm.js terminal panes. */
export function TerminalContextMenu({
  menu,
  canSplit,
  onClose,
  onCopy,
  onPaste,
  onClear,
  onSearch,
  onSplitRight,
  onSplitDown,
  onReset,
  onOpenExternalSsh,
}: TerminalContextMenuProps) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const mac = isMacPlatform();

  useEffect(() => {
    if (!menu) return;
    const handlePointerDown = (ev: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(ev.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') onClose();
    };
    const handleBlur = () => onClose();

    window.addEventListener('mousedown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('blur', handleBlur);
    return () => {
      window.removeEventListener('mousedown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('blur', handleBlur);
    };
  }, [menu, onClose]);

  if (!menu) return null;

  const menuWidth = 224;
  const menuHeight = 290;
  const vw = typeof window !== 'undefined' ? window.innerWidth : 1280;
  const vh = typeof window !== 'undefined' ? window.innerHeight : 800;
  const left = Math.max(8, Math.min(menu.x, vw - menuWidth - 8));
  const top = Math.max(8, Math.min(menu.y, vh - menuHeight - 8));

  const items: MenuItemDef[] = [
    {
      id: 'copy',
      label: '复制',
      shortcut: mac ? '⌘C' : 'Ctrl+Shift+C',
      icon: ClipboardCopy,
      disabled: !menu.hasSelection,
      onClick: onCopy,
    },
    {
      id: 'paste',
      label: '粘贴',
      shortcut: mac ? '⌘V' : 'Ctrl+Shift+V',
      icon: ClipboardPaste,
      onClick: onPaste,
    },
    {
      id: 'search',
      label: '查找',
      shortcut: mac ? '⌘F' : 'Ctrl+Shift+F',
      icon: Search,
      onClick: onSearch,
    },
    {
      id: 'clear',
      label: '清屏',
      shortcut: mac ? '⌘K' : 'Ctrl+L',
      icon: Eraser,
      onClick: onClear,
    },
    {
      id: 'split-right',
      label: '向右分屏',
      shortcut: 'Ctrl+Shift+D',
      icon: Columns2,
      disabled: !canSplit,
      dividerBefore: true,
      onClick: onSplitRight,
    },
    {
      id: 'split-down',
      label: '向下分屏',
      shortcut: 'Ctrl+Shift+E',
      icon: Rows2,
      disabled: !canSplit,
      onClick: onSplitDown,
    },
    {
      id: 'reset',
      label: '重置终端',
      icon: RotateCcw,
      dividerBefore: true,
      onClick: onReset,
    },
    ...(onOpenExternalSsh
      ? [
          {
            id: 'external-ssh',
            label: '外部原生终端 SSH',
            icon: ExternalLink,
            onClick: onOpenExternalSsh,
          },
        ]
      : []),
  ];

  return (
    <div
      ref={menuRef}
      role="menu"
      style={{ left, top, width: menuWidth }}
      className="strata-term-menu fixed z-50 rounded-lg border border-[var(--border-hairline)] bg-[var(--surface-card)] p-1 shadow-xl backdrop-blur-md"
      onContextMenu={(ev) => ev.preventDefault()}
    >
      {items.map((item) => {
        const Icon = item.icon;
        return (
          <div key={item.id}>
            {item.dividerBefore && <div className="my-1 h-px bg-[var(--border-hairline)]" />}
            <button
              type="button"
              role="menuitem"
              disabled={item.disabled}
              onClick={() => {
                onClose();
                item.onClick();
              }}
              className={cn(
                'flex w-full items-center justify-between gap-2 rounded-md px-2.5 py-1.5 text-left text-xs transition-colors',
                item.disabled
                  ? 'cursor-not-allowed text-[var(--text-tertiary)] opacity-50'
                  : 'text-[var(--text-primary)] hover:bg-[var(--accent)]/15 hover:text-[var(--accent)]',
              )}
            >
              <span className="flex items-center gap-2">
                <Icon className="size-3.5 shrink-0 opacity-80" />
                <span>{item.label}</span>
              </span>
              {item.shortcut && (
                <span className="font-mono text-[10px] text-[var(--text-secondary)]">
                  {item.shortcut}
                </span>
              )}
            </button>
          </div>
        );
      })}
    </div>
  );
}
