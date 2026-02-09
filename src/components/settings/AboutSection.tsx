import { useCallback, useEffect, useState } from "react";
import {
  Download,
  Copy,
  Loader2,
  RefreshCw,
  Terminal,
  CheckCircle2,
  AlertCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { settingsApi } from "@/lib/api";
import { motion } from "framer-motion";
import appIcon from "@/assets/icons/app-icon.png";

const ONE_CLICK_INSTALL_COMMANDS = `# Claude Code
npm install -g @anthropic-ai/claude-code
# Codex
npm install -g @openai/codex
# Gemini CLI
npm install -g @google/gemini-cli
# OpenCode
npm install -g opencode-ai`;

interface ToolState {
  isLoading: boolean;
  version: string | null;
  latest_version: string | null;
  error: string | null;
}

export function AboutSection() {
  const { t } = useTranslation();
  const [toolStates, setToolStates] = useState<Record<string, ToolState>>({});
  const [installingTool, setInstallingTool] = useState<string | null>(null);

  // 初始化所有工具为加载状态
  useEffect(() => {
    const tools = ["nodejs", "claude", "codex", "gemini", "opencode"];
    setToolStates(
      Object.fromEntries(
        tools.map((tool) => [
          tool,
          { isLoading: true, version: null, latest_version: null, error: null },
        ])
      )
    );
  }, []);

  // 加载单个工具版本
  const loadToolVersion = useCallback(async (toolName: string) => {
    setToolStates((prev) => ({
      ...prev,
      [toolName]: { ...prev[toolName], isLoading: true, error: null },
    }));

    try {
      const tools = await settingsApi.getToolVersions();
      const tool = tools.find((t) => t.name === toolName);

      if (tool) {
        setToolStates((prev) => ({
          ...prev,
          [toolName]: {
            isLoading: false,
            version: tool.version,
            latest_version: tool.latest_version,
            error: tool.error,
          },
        }));
      }
    } catch (error) {
      console.error(`[AboutSection] Failed to load ${toolName} version`, error);
      setToolStates((prev) => ({
        ...prev,
        [toolName]: {
          ...prev[toolName],
          isLoading: false,
          error: "Check failed",
        },
      }));
    }
  }, []);

  // 初始加载 - 分别异步检查每个工具
  useEffect(() => {
    const tools = ["nodejs", "claude", "codex", "gemini", "opencode"];
    tools.forEach((tool) => {
      loadToolVersion(tool);
    });
  }, [loadToolVersion]);

  // 刷新所有工具
  const loadToolVersions = useCallback(() => {
    const tools = ["nodejs", "claude", "codex", "gemini", "opencode"];
    tools.forEach((tool) => {
      loadToolVersion(tool);
    });
  }, [loadToolVersion]);

  const handleCopyInstallCommands = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(ONE_CLICK_INSTALL_COMMANDS);
      toast.success(t("settings.installCommandsCopied"), { closeButton: true });
    } catch (error) {
      console.error("[AboutSection] Failed to copy install commands", error);
      toast.error(t("settings.installCommandsCopyFailed"));
    }
  }, [t]);

  const handleInstallTool = useCallback(
    async (toolName: string) => {
      setInstallingTool(toolName);
      try {
        const result = await settingsApi.installTool(toolName);
        if (result.success) {
          toast.success(result.message, { closeButton: true });
          await loadToolVersion(toolName);
        } else {
          toast.error(result.message, { description: result.error });
        }
      } catch (error) {
        console.error("[AboutSection] Install tool failed", error);
        toast.error(t("settings.toolInstallFailed"));
      } finally {
        setInstallingTool(null);
      }
    },
    [t, loadToolVersion],
  );

  const handleUpgradeTool = useCallback(
    async (toolName: string) => {
      setInstallingTool(toolName);
      try {
        const result = await settingsApi.upgradeTool(toolName);
        if (result.success) {
          toast.success(result.message, { closeButton: true });
          await loadToolVersion(toolName);
        } else {
          toast.error(result.message, { description: result.error });
        }
      } catch (error) {
        console.error("[AboutSection] Upgrade tool failed", error);
        toast.error(t("settings.toolUpgradeFailed"));
      } finally {
        setInstallingTool(null);
      }
    },
    [t, loadToolVersion],
  );

  return (
    <motion.section
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3 }}
      className="space-y-6"
    >
      <header className="space-y-1">
        <h3 className="text-sm font-medium">{t("common.about")}</h3>
        <p className="text-xs text-muted-foreground">
          {t("settings.aboutHint")}
        </p>
      </header>

      <motion.div
        initial={{ opacity: 0, scale: 0.98 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.3, delay: 0.1 }}
        className="rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-6 shadow-sm"
      >
        <div className="flex items-center gap-2">
          <img src={appIcon} alt="CLI Switch" className="h-5 w-5" />
          <h4 className="text-lg font-semibold text-foreground">
            CLI Switch
          </h4>
        </div>
      </motion.div>

      <div className="space-y-3">
        <div className="flex items-center justify-between px-1">
          <h3 className="text-sm font-medium">{t("settings.localEnvCheck")}</h3>
          <Button
            size="sm"
            variant="outline"
            className="h-7 gap-1.5 text-xs"
            onClick={loadToolVersions}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {t("common.refresh")}
          </Button>
        </div>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-5 px-1">
          {["nodejs", "claude", "codex", "gemini", "opencode"].map((toolName, index) => {
            const toolState = toolStates[toolName] ?? {
              isLoading: true,
              version: null,
              latest_version: null,
              error: null,
            };
            // Special display names
            const displayName =
              toolName === "nodejs"
                ? "Node.js"
                : toolName === "opencode"
                  ? "OpenCode"
                  : toolName.charAt(0).toUpperCase() + toolName.slice(1);
            const title = toolState.version || toolState.error || t("common.unknown");

            return (
              <motion.div
                key={toolName}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.3, delay: 0.15 + index * 0.05 }}
                whileHover={{ scale: 1.02 }}
                className="flex flex-col gap-2 rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-4 shadow-sm transition-colors hover:border-primary/30"
              >
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Terminal className="h-4 w-4 text-muted-foreground" />
                    <span className="text-sm font-medium">{displayName}</span>
                  </div>
                  {toolState.isLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                  ) : toolState.version ? (
                    <div className="flex items-center gap-1.5">
                      {toolState.latest_version &&
                        toolState.version !== toolState.latest_version && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-yellow-500/10 text-yellow-600 dark:text-yellow-400 border border-yellow-500/20">
                            {toolState.latest_version}
                          </span>
                        )}
                      <CheckCircle2 className="h-4 w-4 text-green-500" />
                    </div>
                  ) : (
                    <AlertCircle className="h-4 w-4 text-yellow-500" />
                  )}
                </div>
                <div
                  className="text-xs font-mono text-muted-foreground truncate"
                  title={title}
                >
                  {toolState.isLoading
                    ? t("common.loading")
                    : toolState.version
                      ? toolState.version
                      : toolState.error || t("common.notInstalled")}
                </div>
                <div className="flex items-center gap-2 mt-2">
                  {toolName === "nodejs" ? (
                    <span className="text-[10px] text-muted-foreground text-center w-full">
                      {t("settings.visitNodejsOrg")}
                    </span>
                  ) : !toolState.version ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-7 text-xs w-full"
                      onClick={() => handleInstallTool(toolName)}
                      disabled={installingTool === toolName}
                    >
                      {installingTool === toolName ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <Download className="h-3 w-3" />
                      )}
                      {t("settings.install")}
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-7 text-xs w-full"
                      onClick={() => handleUpgradeTool(toolName)}
                      disabled={installingTool === toolName}
                    >
                      {installingTool === toolName ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <RefreshCw className="h-3 w-3" />
                      )}
                      {t("settings.upgrade")}
                    </Button>
                  )}
                </div>
              </motion.div>
            );
          })}
        </div>
      </div>

      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3, delay: 0.3 }}
        className="space-y-3"
      >
        <h3 className="text-sm font-medium px-1">
          {t("settings.oneClickInstall")}
        </h3>
        <div className="rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-4 space-y-3 shadow-sm">
          <div className="flex items-center justify-between gap-2">
            <p className="text-xs text-muted-foreground">
              {t("settings.oneClickInstallHint")}
            </p>
            <Button
              size="sm"
              variant="outline"
              onClick={handleCopyInstallCommands}
              className="h-7 gap-1.5 text-xs"
            >
              <Copy className="h-3.5 w-3.5" />
              {t("common.copy")}
            </Button>
          </div>
          <pre className="text-xs font-mono bg-background/80 px-3 py-2.5 rounded-lg border border-border/60 overflow-x-auto">
            {ONE_CLICK_INSTALL_COMMANDS}
          </pre>
        </div>
      </motion.div>
    </motion.section>
  );
}
