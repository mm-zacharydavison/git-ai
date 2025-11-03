import * as vscode from "vscode";
import { AIEditManager } from "./ai-edit-manager";
import { IDEHostConfiguration } from "./utils/host-kind";
import { TAB_AI_COMPLETION_COMMANDS } from "./consts";

export class AITabEditManager {
  private context: vscode.ExtensionContext;
  private ideHostConfig: IDEHostConfiguration;
  private aiEditManager: AIEditManager;
  private registration: vscode.Disposable | undefined;
  private restoring = false; // guards against re-entrancy during re-register

  constructor(context: vscode.ExtensionContext, ideHostConfig: IDEHostConfiguration, aiEditManager: AIEditManager) {
    this.context = context;
    this.ideHostConfig = ideHostConfig;
    this.aiEditManager = aiEditManager;
  }

  enableIfSupported(): void {
    if (this.isSupportedIDEHost()) {
      console.log(`[git-ai] Enabling AI tab detection for ${this.ideHostConfig.kind}`);
      this.registerOverride();
      return;
    }
    console.log(`[git-ai] AI tab detection not supported for ${this.ideHostConfig.kind}`);
  }

  beforeHook(args: any[]) {
    // e.g., remember cursor position / active doc
    console.debug('[acceptCursorTabSuggestion] before', args);
  }

  afterHook(result: unknown) {
    // e.g., inspect last edit or fire your own event
    console.debug('[acceptCursorTabSuggestion] after', result);
  }

  registerOverride() {
    const disp = vscode.commands.registerCommand(this.getTabAcceptedCommand(), async (...args: any[]) => {
      // If we're currently re-registering (restoring), just bail to avoid loops.
      if (this.restoring) {
        return;
      }

      // Unregister our override so executing the same command calls the previous handler.
      try {
        this.registration?.dispose();
        this.registration = undefined;
      } catch { /* ignore */ }

      try {
        await this.beforeHook(args);

        // Call the "original" command implementation (the previously registered handler).
        const result = await vscode.commands.executeCommand(this.getTabAcceptedCommand(), ...args);

        await this.afterHook(result);
        return result;
      } finally {
        // Always restore our override so future executions flow through us again.
        try {
          this.restoring = true;
          this.registration = this.registerOverride();
        } finally {
          this.restoring = false;
        }
      }
    });

    // Keep it in extension subscriptions so VS Code cleans up on deactivate.
    this.context.subscriptions.push(disp);
    return disp;
  }

  isSupportedIDEHost(): boolean {
    return TAB_AI_COMPLETION_COMMANDS[this.ideHostConfig.kind] !== undefined;
  }

  getTabAcceptedCommand(): string {
    let command = TAB_AI_COMPLETION_COMMANDS[this.ideHostConfig.kind];
    if (!command) {
      throw new Error(`Unsupported IDE host kind: ${this.ideHostConfig.kind}`);
    }
    return command;
  }
}