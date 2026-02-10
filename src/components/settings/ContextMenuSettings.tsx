import { useState, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Loader2, Check, Trash2, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { isWindows, isMac } from "@/lib/platform";

export interface ContextMenuSettingsProps {
  onProvidersChange?: () => void;
}

/**
 * Windows/macOS 文件夹右键菜单设置组件
 *
 * 功能：
 * - 注册/注销右键菜单
 * - 检测注册状态
 * - 供应商变更时自动重新注册
 */
export function ContextMenuSettings({
  onProvidersChange,
}: ContextMenuSettingsProps) {
  const [isRegistered, setIsRegistered] = useState<boolean | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRegistering, setIsRegistering] = useState(false);
  const [isUnregistering, setIsUnregistering] = useState(false);

  // 检查注册状态
  const checkRegistrationStatus = async () => {
    try {
      setIsLoading(true);
      const registered = await settingsApi.isContextMenuRegistered();
      setIsRegistered(registered);
    } catch (error) {
      console.error("检查右键菜单状态失败:", error);
      // 非支持平台会返回错误，这是正常的
      setIsRegistered(false);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    if (isWindows() || isMac()) {
      checkRegistrationStatus();
    }
  }, []);

  // 注册右键菜单
  const handleRegister = async () => {
    try {
      console.log("[ContextMenu] 开始注册右键菜单");
      setIsRegistering(true);
      await settingsApi.registerContextMenu();
      console.log("[ContextMenu] 注册成功");
      // 从注册表重新检测状态
      await checkRegistrationStatus();
      toast.success("右键菜单注册成功");
    } catch (error) {
      console.error("[ContextMenu] 注册失败:", error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      // 检查是否是 UAC 弹窗提示
      if (errorMsg.includes("UAC") || errorMsg.includes("确认")) {
        toast.info("请在弹出的 UAC 窗口中确认", {
          duration: 5000,
        });
      } else {
        toast.error(`注册失败: ${errorMsg}`);
      }
    } finally {
      setIsRegistering(false);
    }
  };

  // 注销右键菜单
  const handleUnregister = async () => {
    try {
      setIsUnregistering(true);
      await settingsApi.unregisterContextMenu();
      // 从注册表重新检测状态
      await checkRegistrationStatus();
      toast.success("右键菜单已注销");
    } catch (error) {
      console.error("注销右键菜单失败:", error);
      toast.error(`注销失败: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setIsUnregistering(false);
    }
  };

  // 刷新状态（供应商变更后使用）
  const handleRefresh = () => {
    checkRegistrationStatus();
  };

  // 重启 Finder（仅 macOS）
  const handleRestartFinder = async () => {
    try {
      await settingsApi.restartFinder();
      toast.success("访达已重启，请重新检查右键菜单");
      // 等待后重新检查状态
      setTimeout(() => checkRegistrationStatus(), 2000);
    } catch (error) {
      console.error("重启访达失败:", error);
      toast.error(`重启失败: ${error instanceof Error ? error.message : String(error)}`);
    }
  };

  // 当供应商列表变更时，如果已注册则自动重新注册
  useEffect(() => {
    if (isRegistered === true && onProvidersChange) {
      // 供应商变更后自动重新注册菜单
      settingsApi.registerContextMenu().catch((error) => {
        console.error("自动重新注册右键菜单失败:", error);
      });
    }
  }, [onProvidersChange]);

  // 非 Windows/macOS 平台不显示
  if (!isWindows() && !isMac()) {
    return null;
  }

  return (
    <section className="space-y-3">
      <header className="space-y-1">
        <h3 className="text-sm font-medium">右键菜单</h3>
        <p className="text-xs text-muted-foreground">
          在文件夹右键菜单 CLI Switch 中添加"打开 CLI 终端"选项
        </p>
      </header>

      <div className="flex items-center gap-2">
        {isLoading ? (
          <Button variant="outline" size="sm" disabled>
            <Loader2 className="w-4 h-4 mr-2 animate-spin" />
            检测中...
          </Button>
        ) : isRegistered ? (
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={handleUnregister}
              disabled={isUnregistering}
            >
              {isUnregistering ? (
                <Loader2 className="w-4 h-4 mr-2 animate-spin" />
              ) : (
                <Trash2 className="w-4 h-4 mr-2" />
              )}
              注销
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleRefresh}
              disabled={isRegistering}
            >
              <RefreshCw className="w-4 h-4 mr-2" />
              刷新
            </Button>
            {isMac() && (
              <Button
                variant="ghost"
                size="sm"
                onClick={handleRestartFinder}
              >
                重启访达
              </Button>
            )}
            <div className="flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
              <Check className="w-3 h-3" />
              <span>已注册</span>
            </div>
          </>
        ) : (
          <>
            <Button
              size="sm"
              onClick={handleRegister}
              disabled={isRegistering}
            >
              {isRegistering ? (
                <Loader2 className="w-4 h-4 mr-2 animate-spin" />
              ) : null}
              注册
            </Button>
            {isMac() && (
              <Button
                variant="ghost"
                size="sm"
                onClick={handleRestartFinder}
              >
                重启访达
              </Button>
            )}
          </>
        )}
      </div>

      <p className="text-xs text-muted-foreground">
        注册后，在文件夹上右键可看到"在此处打开 Claude 终端"等选项。
        添加或删除供应商后需要重新注册。
      </p>
    </section>
  );
}
