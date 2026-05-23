import { execFile } from "node:child_process";
import { access } from "node:fs/promises";
import { join } from "node:path";
import { promisify } from "node:util";

import streamDeck, {
  action,
  type KeyAction,
  SingletonAction,
  type KeyDownEvent,
  type WillAppearEvent,
} from "@elgato/streamdeck";

const execFileAsync = promisify(execFile);
const PLUGIN_UUID = "dev.gomeng.lg-tv-display-switcher";
const APP_NAME = "LG-TV-Display-Switcher";
const APP_EXE = "LG-TV-Display-Switcher.exe";
const COMPANION_TIMEOUT_MS = 8_000;
const RELEASE_URL =
  "https://github.com/gomeng-dev/LG-TV-Display-Switcher/releases/latest";

type CompanionCommand =
  | "apply-tv-mode"
  | "apply-pc-mode"
  | "toggle-display-mode"
  | "toggle-tv-power"
  | "toggle-auto-switch"
  | "status";

type DisplayMode = "pc" | "tv" | "unknown";

type CompanionResult = {
  ok: boolean;
  status: string;
  tvOn: boolean | null;
  displayMode: DisplayMode;
  autoSwitchDisplays: boolean;
  installRequired: boolean;
  updateRequired: boolean;
  error: string | null;
};

type ActionConfig = {
  uuid: string;
  command: CompanionCommand;
  defaultTitle: string;
  successTitle: (result: CompanionResult) => string;
};

const rememberedDisplayModes = new Map<string, DisplayMode>();
let companionCliUnavailable = false;
let commandInFlight = false;

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
  if (companionCliUnavailable) {
    return updateRequiredResult();
  }

  const appPath = await findCompanionApp();
  if (!appPath) {
    return {
      ok: false,
      status: "App missing",
      tvOn: null,
      displayMode: "unknown",
      autoSwitchDisplays: false,
      installRequired: true,
      updateRequired: false,
      error: "LG-TV-Display-Switcher is not installed.",
    };
  }

  try {
    const { stdout } = await execFileAsync(
      appPath,
      ["--streamdeck", command, "--json"],
      {
        timeout: COMPANION_TIMEOUT_MS,
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
      try {
        return parseCompanionOutput(output);
      } catch {
        companionCliUnavailable = true;
        return updateRequiredResult();
      }
    }

    companionCliUnavailable = true;
    return updateRequiredResult(
      error instanceof Error ? error.message : String(error),
    );
  }
}

function updateRequiredResult(error?: string): CompanionResult {
  return {
    ok: false,
    status: "Update app required",
    tvOn: null,
    displayMode: "unknown",
    autoSwitchDisplays: false,
    installRequired: false,
    updateRequired: true,
    error:
      error ||
      "The installed companion app does not support the Stream Deck CLI. Please install the latest LG-TV-Display-Switcher.",
  };
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
    displayMode:
      parsed.displayMode === "pc" || parsed.displayMode === "tv"
        ? parsed.displayMode
        : "unknown",
    autoSwitchDisplays: Boolean(parsed.autoSwitchDisplays),
    installRequired: Boolean(parsed.installRequired),
    updateRequired: Boolean(parsed.updateRequired),
    error: parsed.error ? String(parsed.error) : null,
  };
}

function displayModeTitle(result: CompanionResult): string {
  return result.displayMode === "tv" ? "TV\nMode" : "PC\nMode";
}

function tvPowerTitle(result: CompanionResult): string {
  if (/wake tv packet sent/i.test(result.status)) {
    return "TV\nWaking";
  }

  return result.tvOn === true
    ? "TV\nOn"
    : result.tvOn === false
      ? "TV\nOff"
      : "TV\nPower";
}

function failureTitle(result: CompanionResult): string {
  const message = `${result.error ?? ""} ${result.status ?? ""}`;
  if (/\b(tv)?mac\b|tvmac/i.test(message)) {
    return "No\nMAC";
  }
  if (/tv is not on|tv off|not on/i.test(message)) {
    return "TV\nOff";
  }
  if (/not configured|config/i.test(message)) {
    return "Config";
  }
  return "Failed";
}

function actionKey(ev: KeyDownEvent | WillAppearEvent): string {
  return String(ev.action.id);
}

async function updateDisplayModeAction(
  ev: KeyDownEvent | WillAppearEvent,
  result: CompanionResult,
): Promise<void> {
  if (result.tvOn === false) {
    if ("setState" in ev.action) {
      await (ev.action as KeyAction).setState(0);
    }
    await ev.action.setTitle("TV\nOff");
    return;
  }

  let displayMode = result.displayMode;
  if (displayMode === "unknown") {
    displayMode = rememberedDisplayModes.get(actionKey(ev)) ?? "pc";
  } else {
    rememberedDisplayModes.set(actionKey(ev), displayMode);
  }

  if ("setState" in ev.action) {
    await (ev.action as KeyAction).setState(displayMode === "tv" ? 1 : 0);
  }
  await ev.action.setTitle(displayMode === "tv" ? "TV\nMode" : "PC\nMode");
}

async function toggleDisplayMode(ev: KeyDownEvent): Promise<CompanionResult> {
  const current = await runCompanion("status");
  if (current.installRequired || current.updateRequired) {
    return current;
  }

  if (current.tvOn !== true) {
    return {
      ...current,
      ok: false,
      error: "TV is off",
    };
  }

  const remembered = rememberedDisplayModes.get(actionKey(ev));
  const mode =
    current.displayMode === "unknown" ? remembered ?? "pc" : current.displayMode;
  const command: CompanionCommand =
    mode === "tv" ? "apply-pc-mode" : "apply-tv-mode";
  const result = await runCompanion(command);

  if (result.ok) {
    const nextMode = command === "apply-pc-mode" ? "pc" : "tv";
    rememberedDisplayModes.set(
      actionKey(ev),
      result.displayMode === "unknown" ? nextMode : result.displayMode,
    );
    return {
      ...result,
      displayMode: result.displayMode === "unknown" ? nextMode : result.displayMode,
    };
  }

  return result;
}

async function handleUnavailableApp(
  ev: KeyDownEvent,
  result: CompanionResult,
): Promise<void> {
  await ev.action.setTitle(result.updateRequired ? "Update app" : "Install app");
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
      if (result.updateRequired) {
        await ev.action.setTitle("Update app");
        return;
      }
      await ev.action.setTitle(
        result.autoSwitchDisplays ? "Auto\nOn" : "Auto\nOff",
      );
      return;
    }

    if (this.config.command === "toggle-display-mode") {
      const result = await runCompanion("status");
      if (result.updateRequired) {
        await ev.action.setTitle("Update app");
        return;
      }
      await updateDisplayModeAction(ev, result);
      return;
    }

    await ev.action.setTitle(this.config.defaultTitle);
  }

  override async onKeyDown(ev: KeyDownEvent): Promise<void> {
    if (commandInFlight) {
      await ev.action.showAlert();
      return;
    }

    commandInFlight = true;
    try {
      const result =
        this.config.command === "toggle-display-mode"
          ? await toggleDisplayMode(ev)
          : await runCompanion(this.config.command);
      if (result.installRequired || result.updateRequired) {
        await handleUnavailableApp(ev, result);
        return;
      }

      if (!result.ok) {
        await ev.action.setTitle(failureTitle(result));
        await ev.action.showAlert();
        return;
      }

      if (this.config.command === "toggle-display-mode") {
        await updateDisplayModeAction(ev, result);
      } else {
        await ev.action.setTitle(this.config.successTitle(result));
      }
      await ev.action.showOk();
    } finally {
      commandInFlight = false;
    }
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
  successTitle: tvPowerTitle,
});

registerAction({
  uuid: `${PLUGIN_UUID}.display-mode-toggle`,
  command: "toggle-display-mode",
  defaultTitle: "PC\nMode",
  successTitle: displayModeTitle,
});

registerAction({
  uuid: `${PLUGIN_UUID}.toggle-auto-switch`,
  command: "toggle-auto-switch",
  defaultTitle: "Auto",
  successTitle: (result) =>
    result.autoSwitchDisplays ? "Auto\nOn" : "Auto\nOff",
});

void streamDeck.connect();
