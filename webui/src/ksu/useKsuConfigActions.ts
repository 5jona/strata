import type { BridgeClient } from '@gojona/bridge';
import type { Dispatch, SetStateAction } from 'react';
import {
  type ContainerConfig,
  type ContainerConfigPatchParams,
  type ContainerStatusSnapshot,
  STRATA_METHODS,
  type WebStatusResult,
} from '../types.ts';
import { invokeStrataKsuRpc, readStrataKsuWebToken } from './ksuStrataClient.ts';

export interface UseKsuConfigActionsOptions {
  bridge: BridgeClient;
  activeContainer: ContainerStatusSnapshot | null;
  setBusyKey: (key: string | null) => void;
  setContainerConfig: Dispatch<SetStateAction<ContainerConfig | null>>;
  setSshKeys: Dispatch<SetStateAction<string[]>>;
  setWebStatus: Dispatch<SetStateAction<WebStatusResult | null>>;
  setPairingToken: Dispatch<SetStateAction<string | null>>;
  refreshAll: () => Promise<void>;
  pushToast: (message: string, tone?: 'info' | 'success' | 'danger') => void;
}

/** Configuration, SSH key management, and PC pairing actions for the KSU WebUI. */
export function useKsuConfigActions({
  bridge,
  activeContainer,
  setBusyKey,
  setContainerConfig,
  setSshKeys,
  setWebStatus,
  setPairingToken,
  refreshAll,
  pushToast,
}: UseKsuConfigActionsOptions) {
  const errMsg = (err: unknown) => (err instanceof Error ? err.message : String(err));

  const patchConfig = async (patch: ContainerConfigPatchParams): Promise<boolean> => {
    if (!activeContainer) return false;
    setBusyKey('config:save');
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_CONFIG_PATCH, {
        ...patch,
        container: activeContainer.id,
      });
      setContainerConfig(res.config);
      pushToast('配置已保存，重启容器后生效', 'success');
      await refreshAll();
      return true;
    } catch (err) {
      pushToast(errMsg(err), 'danger');
      return false;
    } finally {
      setBusyKey(null);
    }
  };

  const addSshKey = async (publicKey: string): Promise<boolean> => {
    if (!activeContainer) return false;
    setBusyKey('ssh:add');
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_ADD, {
        container: activeContainer.id,
        publicKey,
      });
      setSshKeys(res.keys);
      pushToast(res.added > 0 ? `已添加公钥（共 ${res.keys.length} 条）` : '公钥已存在', 'success');
      return true;
    } catch (err) {
      pushToast(errMsg(err), 'danger');
      return false;
    } finally {
      setBusyKey(null);
    }
  };

  const removeSshKey = async (fingerprintOrKey: string): Promise<boolean> => {
    if (!activeContainer) return false;
    setBusyKey('ssh:remove');
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.CONTAINER_SSH_KEY_REMOVE, {
        id: activeContainer.id,
        fingerprintOrKey,
      });
      setSshKeys(res.keys);
      pushToast(
        res.removed > 0 ? '公钥已删除' : '未匹配到指定公钥',
        res.removed > 0 ? 'success' : 'info',
      );
      return res.removed > 0;
    } catch (err) {
      pushToast(errMsg(err), 'danger');
      return false;
    } finally {
      setBusyKey(null);
    }
  };

  const toggleLan = async (lanEnabled: boolean) => {
    setBusyKey('web:lan');
    try {
      const next = await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_CONFIGURE, { lanEnabled });
      setWebStatus(next);
      pushToast(lanEnabled ? '已开启局域网配对' : '已关闭局域网配对（仅限本机访问）', 'success');
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const readToken = async () => {
    setBusyKey('web:read');
    try {
      const res = await readStrataKsuWebToken(bridge);
      setPairingToken(res.token);
      pushToast('已读取当前配对密钥', 'success');
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  const rotateToken = async () => {
    setBusyKey('web:rotate');
    try {
      const res = await invokeStrataKsuRpc(bridge, STRATA_METHODS.WEB_ROTATE_TOKEN);
      setPairingToken(res.token);
      pushToast(`配对密钥已刷新（已断开 ${res.sessionsRevoked} 个旧连接）`, 'success');
    } catch (err) {
      pushToast(errMsg(err), 'danger');
    } finally {
      setBusyKey(null);
    }
  };

  return { patchConfig, addSshKey, removeSshKey, toggleLan, readToken, rotateToken };
}
