import * as vscode from "vscode";
import * as path from "path";
import { exec, spawn } from "child_process";

class AIEditManager {
  private workspaceBaseStoragePath: string | null = null;
  private gitAiVersion: string | null = null;
  private hasShownGitAiMissingMessage = false;
  private lastHumanCheckpointAt: Date | null = null;
  private pendingSaves = new Map<string, {
    timestamp: number;
    timer: NodeJS.Timeout;
  }>();
  private snapshotOpenEvents = new Map<string, {
    timestamp: number;
    count: number;
    uri: vscode.Uri;
  }>();
  private readonly SAVE_EVENT_DEBOUNCE_WINDOW_MS = 300;
  private readonly HUMAN_CHECKPOINT_DEBOUNCE_MS = 500;

  constructor(context: vscode.ExtensionContext) {    
    if (context.storageUri?.fsPath) {
      this.workspaceBaseStoragePath = path.dirname(context.storageUri.fsPath);  
    } else {
      // No workspace active (extension will be re-activated when a workspace is opened)
      console.warn('[git-ai] No workspace storage URI available');
    }
  }

  public handleSaveEvent(doc: vscode.TextDocument): void {
    const filePath = doc.uri.fsPath;

    // Clear any existing timer for this file
    const existing = this.pendingSaves.get(filePath);
    if (existing) {
      clearTimeout(existing.timer);
    }

    // Set up new debounce timer
    const timer = setTimeout(() => {
      this.evaluateSaveForCheckpoint(filePath);
    }, this.SAVE_EVENT_DEBOUNCE_WINDOW_MS);

    this.pendingSaves.set(filePath, {
      timestamp: Date.now(),
      timer
    });

    console.log('[git-ai] AIEditManager: Save event tracked for', filePath);
  }

  public handleOpenEvent(doc: vscode.TextDocument): void {
    if (doc.uri.scheme === "chat-editing-snapshot-text-model") {
      const filePath = doc.uri.fsPath;
      const now = Date.now();

      const existing = this.snapshotOpenEvents.get(filePath);
      if (existing) {
        existing.count++;
        existing.timestamp = now;
      } else {
        this.snapshotOpenEvents.set(filePath, {
          timestamp: now,
          count: 1,
          uri: doc.uri // TODO Should we just let first writer wins for URI?
        });
      }

      console.log('[git-ai] AIEditManager: Snapshot open event tracked for', filePath, 'count:', this.snapshotOpenEvents.get(filePath)?.count);
    }
  }

  public handleCloseEvent(doc: vscode.TextDocument): void {
    if (doc.uri.scheme === "chat-editing-snapshot-text-model") {
      console.log('[git-ai] AIEditManager: Snapshot close event detected, triggering human checkpoint');
      this.checkpoint("human");
    }
  }

  private evaluateSaveForCheckpoint(filePath: string): void {
    const saveInfo = this.pendingSaves.get(filePath);
    if (!saveInfo) {
      return;
    }

    const snapshotInfo = this.snapshotOpenEvents.get(filePath);

    // Check if we have 1+ valid snapshot open events within the debounce window
    if (snapshotInfo && snapshotInfo.count >= 1 && snapshotInfo.uri?.query) {
      try {
        if (!this.workspaceBaseStoragePath) {
          throw new Error('No workspace base storage path found');
        }
        const params = JSON.parse(snapshotInfo.uri.query);
        if (!params.sessionId || !params.requestId) {
          throw new Error('Missing required parameters in snapshot URI query');
        }
        let sessionId = params.sessionId || null;
        let requestId = params.requestId || null;
        let chatSessionPath = path.join(this.workspaceBaseStoragePath, 'chatSessions', sessionId+'.json');
        // Get the workspace folder for the file, fallback to workspaceBaseStoragePath if not found
        let workspaceFolder = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(filePath));
        if (!workspaceFolder) {
          throw new Error('No workspace folder found for file path: ' + filePath);
        }
        console.log('[git-ai] AIEditManager: AI edit detected for', filePath, '- triggering AI checkpoint (sessionId:', sessionId, ', requestId:', requestId, ', chatSessionPath:', chatSessionPath, ', workspaceFolder:', workspaceFolder.uri.fsPath, ')');
        this.checkpoint("ai", JSON.stringify({
          chatSessionPath,
          sessionId,
          requestId,
          workspaceFolder: workspaceFolder.uri.fsPath,
        }));
      } catch (e) {
        console.error('[git-ai] AIEditManager: Failed to parse snapshot URI query as JSON. Unable to trigger AI checkpoint', e);
      }
    } else {
      console.log('[git-ai] AIEditManager: No AI pattern detected for', filePath, '- triggering human checkpoint');
      this.checkpoint("human");
    }

    // Cleanup
    this.pendingSaves.delete(filePath);
    this.snapshotOpenEvents.delete(filePath);
  }

  public triggerInitialHumanCheckpoint(): void {
    console.log('[git-ai] AIEditManager: Triggering initial human checkpoint');
    this.checkpoint("human");
  }

  async checkpoint(author: "human" | "ai", hookInput?: string): Promise<boolean> {
    if (!(await this.checkGitAi())) {
      return false;
    }

    // Throttle human checkpoints
    if (author === "human") {
      const now = new Date();
      if (this.lastHumanCheckpointAt && (now.getTime() - this.lastHumanCheckpointAt.getTime()) < this.HUMAN_CHECKPOINT_DEBOUNCE_MS) {
        console.log('[git-ai] AIEditManager: Skipping human checkpoint due to debounce');
        return false;
      }
      this.lastHumanCheckpointAt = now;
    }
    
    return new Promise<boolean>((resolve, reject) => {
      let workspaceRoot: string | undefined;

      const activeEditor = vscode.window.activeTextEditor;
      if (activeEditor) {
        const documentUri = activeEditor.document.uri;
        const workspaceFolder = vscode.workspace.getWorkspaceFolder(documentUri);
        if (workspaceFolder) {
          workspaceRoot = workspaceFolder.uri.fsPath;
        }
      }

      if (!workspaceRoot) {
        workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
      }

      if (!workspaceRoot) {
        vscode.window.showErrorMessage("No workspace root found");
        resolve(false);
        return;
      }

      const args = ["checkpoint"];
      if (author === "ai") {
        args.push("github-copilot");
      }
      if (hookInput) {
        args.push("--hook-input", "stdin");
      }

      const proc = spawn("git-ai", args, { cwd: workspaceRoot });

      let stdout = "";
      let stderr = "";

      proc.stdout.on("data", (data) => {
        stdout += data.toString();
      });

      proc.stderr.on("data", (data) => {
        stderr += data.toString();
      });

      proc.on("error", (error) => {
        console.error('[git-ai] AIEditManager: Checkpoint error:', error, stdout, stderr);
        vscode.window.showErrorMessage(
          "git-ai checkpoint error: " + error.message + " - " + stdout + " - " + stderr
        );
        resolve(false);
      });

      proc.on("close", (code) => {
        if (code !== 0) {
          console.error('[git-ai] AIEditManager: Checkpoint exited with code:', code, stdout, stderr);
          vscode.window.showErrorMessage(
            "git-ai checkpoint error: exited with code " + code + " - " + stdout + " - " + stderr
          );
          resolve(false);
        } else {
          const config = vscode.workspace.getConfiguration("gitai");
          if (config.get("enableCheckpointLogging")) {
            vscode.window.showInformationMessage(
              "Checkpoint created " + author
            );
          }
          resolve(true);
        }
      });

      if (hookInput) {
        proc.stdin.write(hookInput);
        proc.stdin.end();
      }
    });
  }

  async checkGitAi(): Promise<boolean> {
    if (this.gitAiVersion) {
      return true;
    }
    // TODO Consider only re-checking every X attempts



    return new Promise((resolve) => {
      exec("git-ai --version", (error, stdout, stderr) => {
        if (error) {
          if (!this.hasShownGitAiMissingMessage) {
            // Show startup notification
            vscode.window.showInformationMessage(
              "git-ai not installed. Visit https://github.com/acunniffe/git-ai to install it."
            );
            this.hasShownGitAiMissingMessage = true;
          }
          // not installed. do nothing
          resolve(false);
        } else {
          // Save the version for later use
          this.gitAiVersion = stdout.trim();

          // Show startup notification
          vscode.window.showInformationMessage(
            `🤖 AI Code Detector is now active! (git-ai v${this.gitAiVersion})`
          );
          resolve(true);
        }
      });
    });
  }
}

export function activate(context: vscode.ExtensionContext) {
  console.log('[git-ai] extension activated');

  const aiEditManager = new AIEditManager(context);

  // Trigger initial human checkpoint
  aiEditManager.triggerInitialHumanCheckpoint();

  // Log all initially open files
  vscode.workspace.textDocuments.forEach(doc => {
    console.log('[git-ai] initial open file', doc);
  });

  // Change event
  context.subscriptions.push(
    vscode.workspace.onDidChangeTextDocument((event) => {
      console.log('[git-ai] change event', event);
    })
  );

  // Save event
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      console.log('[git-ai] save event', doc);
      aiEditManager.handleSaveEvent(doc);
    })
  );

  // Open event
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => {
      console.log('[git-ai] open event', doc);
      aiEditManager.handleOpenEvent(doc);
    })
  );

  // Close event
  context.subscriptions.push(
    vscode.workspace.onDidCloseTextDocument((doc) => {
      console.log('[git-ai] close event', doc);
      aiEditManager.handleCloseEvent(doc);
    })
  );

  // Will save event
  context.subscriptions.push(
    vscode.workspace.onWillSaveTextDocument((event) => {
      console.log('[git-ai] will save event', event);
    })
  );

  // Create event
  context.subscriptions.push(
    vscode.workspace.onDidCreateFiles((event) => {
      console.log('[git-ai] create event', event);
    })
  );

  // Delete event
  context.subscriptions.push(
    vscode.workspace.onDidDeleteFiles((event) => {
      console.log('[git-ai] delete event', event);
    })
  );

  // Rename event
  context.subscriptions.push(
    vscode.workspace.onDidRenameFiles((event) => {
      console.log('[git-ai] rename event', event);
    })
  );
}

export function deactivate() {}
