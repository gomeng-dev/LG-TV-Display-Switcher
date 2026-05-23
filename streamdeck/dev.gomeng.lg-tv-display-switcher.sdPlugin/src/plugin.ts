import { execFile } from "node:child_process";
import { access } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import streamDeck, {
  action,
  SingletonAction,
  type KeyDownEvent,
  type WillAppearEvent,
} from "@elgato/streamdeck";

const execFileAsync = promisify(execFile);
const PLUGIN_UUID = "dev.gomeng.lg-tv-display-switcher";
const APP_NAME = "LG-TV-Display-Switcher";
const APP_EXE = "LG-TV-Display-Switcher.exe";
const RELEASE_URL =
  "https://github.com/gomeng-dev/LG-TV-Display-Switcher/releases/latest";

type CompanionCommand =
  | "apply-tv-mode"
  | "apply-pc-mode"
  | "toggle-tv-power"
  | "toggle-auto-switch"
  | "status";

type CompanionResult = {
  ok: boolean;
  status: string;
  tvOn: boolean | null;
  autoSwitchDisplays: boolean;
  installRequired: boolean;
  error: string | null;
};

type ActionConfig = {
  uuid: string;
  command: CompanionCommand;
  defaultTitle: string;
  successTitle: (result: CompanionResult) => string;
};

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function findCompanionApp(): Promise<string | null> {
  const installLocation = await readInstallLocation();
  if (installLocation) {
    const installedPath = join(installLocation, APP_EXE);
    if (await fileExists(installedPath)) {
      return installedPath;
    }
  }

  const localAppData = process.env.LOCALAPPDATA;
  if (localAppData) {
    const fallbackPath = join(localAppData, APP_NAME, APP_EXE);
    if (await fileExists(fallbackPath)) {
      return fallbackPath;
    }
  }

  return null;
}

async function readInstallLocation(): Promise<string | null> {
  try {
    const { stdout } = await execFileAsync(
      "reg.exe",
      [
        "query",
        `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${APP_NAME}`,
        "/v",
        "InstallLocation",
      ],
      { windowsHide: true },
    );

    const match = stdout.match(/InstallLocation\s+REG_SZ\s+(.+)/i);
    return match?.[1]?.trim() || null;
  } catch {
    return null;
  }
}

async function runCompanion(command: CompanionCommand): Promise<CompanionResult> {
  const appPath = await findCompanionApp();
  if (!appPath) {
    return {
      ok: false,
      status: "App missing",
      tvOn: null,
      autoSwitchDisplays: false,
      installRequired: true,
      error: "LG-TV-Display-Switcher is not installed.",
    };
  }

  try {
    const { stdout } = await execFileAsync(
      appPath,
      ["--streamdeck", command, "--json"],
      {
        timeout: 30_000,
        windowsHide: true,
      },
    );
    return parseCompanionOutput(stdout);
  } catch (error) {
    const output =
      typeof error === "object" && error && "stdout" in error
        ? String((error as { stdout?: unknown }).stdout ?? "")
        : "";

    if (output.trim()) {
      return parseCompanionOutput(output);
    }

    return {
      ok: false,
      status: "Command failed",
      tvOn: null,
      autoSwitchDisplays: false,
      installRequired: false,
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

function parseCompanionOutput(output: string): CompanionResult {
  const line = output
    .trim()
    .split(/\r?\n/)
    .reverse()
    .find((value) => value.trim().startsWith("{"));

  if (!line) {
    throw new Error("The companion app did not return JSON.");
  }

  const parsed = JSON.parse(line) as Partial<CompanionResult>;
  return {
    ok: Boolean(parsed.ok),
    status: String(parsed.status ?? ""),
    tvOn:
      typeof parsed.tvOn === "boolean"
        ? parsed.tvOn
        : parsed.tvOn === null
          ? null
          : null,
    autoSwitchDisplays: Boolean(parsed.autoSwitchDisplays),
    installRequired: Boolean(parsed.installRequired),
    error: parsed.error ? String(parsed.error) : null,
  };
}

async function handleMissingApp(ev: KeyDownEvent): Promise<void> {
  await ev.action.setTitle("Install app");
  await ev.action.showAlert();
  await streamDeck.system.openUrl(RELEASE_URL);
}

class CompanionAction extends SingletonAction {
  constructor(private readonly config: ActionConfig) {
    super();
  }

  override async onWillAppear(ev: WillAppearEvent): Promise<void> {
    const appPath = await findCompanionApp();
    if (!appPath) {
      await ev.action.setTitle("App missing");
      return;
    }

    if (this.config.command === "toggle-auto-switch") {
      const result = await runCompanion("status");
      await ev.action.setTitle(
        result.autoSwitchDisplays ? "Auto\nOn" : "Auto\nOff",
      );
      return;
    }

    await ev.action.setTitle(this.config.defaultTitle);
  }

  override async onKeyDown(ev: KeyDownEvent): Promise<void> {
    const result = await runCompanion(this.config.command);
    if (result.installRequired) {
      await handleMissingApp(ev);
      return;
    }

    if (!result.ok) {
      await ev.action.setTitle(result.error || "Failed");
      await ev.action.showAlert();
      return;
    }

    await ev.action.setTitle(this.config.successTitle(result));
    await ev.action.showOk();
  }
}

function registerAction(config: ActionConfig): void {
  @action({ UUID: config.uuid })
  class RegisteredAction extends CompanionAction {
    constructor() {
      super(config);
    }
  }

  streamDeck.actions.registerAction(new RegisteredAction());
}

registerAction({
  uuid: `${PLUGIN_UUID}.tv-power-toggle`,
  command: "toggle-tv-power",
  defaultTitle: "TV\nPower",
  successTitle: (result) =>
    result.tvOn === true ? "TV\nOn" : result.tvOn === false ? "TV\nOff" : "TV\nPower",
});

registerAction({
  uuid: `${PLUGIN_UUID}.apply-tv-mode`,
  command: "apply-tv-mode",
  defaultTitle: "TV\nMode",
  successTitle: () => "TV\nMode",
});

registerAction({
  uuid: `${PLUGIN_UUID}.apply-pc-mode`,
  command: "apply-pc-mode",
  defaultTitle: "PC\nMode",
  successTitle: () => "PC\nMode",
});

registerAction({
  uuid: `${PLUGIN_UUID}.toggle-auto-switch`,
  command: "toggle-auto-switch",
  defaultTitle: "Auto",
  successTitle: (result) =>
    result.autoSwitchDisplays ? "Auto\nOn" : "Auto\nOff",
});

await streamDeck.connect();
