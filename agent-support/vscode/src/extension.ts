import * as vscode from "vscode";
import { AIEditManager } from "./ai-edit-manager";
import { detectIDEHost, IDEHostKindVSCode } from "./utils/host-kind";
import { AITabEditManager } from "./ai-tab-edit-manager";

export function activate(context: vscode.ExtensionContext) {
  console.log('[git-ai] extension activated');

  const ideHostCfg = detectIDEHost();

  const aiEditManager = new AIEditManager(context);

  const aiTabEditManager = new AITabEditManager(context, ideHostCfg, aiEditManager);

  aiTabEditManager.enableIfSupported();

  if (ideHostCfg.kind == IDEHostKindVSCode) {
    // Trigger initial human checkpoint
    aiEditManager.triggerInitialHumanCheckpoint();

    // Save event
    context.subscriptions.push(
      vscode.workspace.onDidSaveTextDocument((doc) => {
        aiEditManager.handleSaveEvent(doc);
      })
    );

    // Open event
    context.subscriptions.push(
      vscode.workspace.onDidOpenTextDocument((doc) => {
        aiEditManager.handleOpenEvent(doc);
      })
    );

    // Close event
    context.subscriptions.push(
      vscode.workspace.onDidCloseTextDocument((doc) => {
        aiEditManager.handleCloseEvent(doc);
      })
    );
  }
}

export function deactivate() { }
