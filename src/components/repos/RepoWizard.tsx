// src/components/repos/RepoWizard.tsx
//
// Four-step in-page wizard for repo registration:
//   1. Pick directory
//   2. Name (defaults to basename, must be non-empty)
//   3. Patterns (gitignore import checkbox + PatternEditor)
//   4. Preview and confirm (file list + register)

import { useState, useEffect } from "react";
import { ArrowLeft, ArrowRight, FolderOpen, Loader2, Check } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  pickDirectory,
  repoGitignorePatterns,
  repoPreview,
  repoAdd,
} from "@/lib/tauri";
import { isCommandError } from "@/lib/types";
import { PatternEditor } from "@/components/repos/PatternEditor";

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface RepoWizardProps {
  onCancel: () => void;
  onComplete: (repoId: number) => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function RepoWizard({ onCancel, onComplete }: RepoWizardProps) {
  // Step tracking
  const [step, setStep] = useState(1);

  // Step 1: directory
  const [dirPath, setDirPath] = useState("");

  // Step 2: name
  const [name, setName] = useState("");

  // Step 3: patterns
  const [importGitignore, setImportGitignore] = useState(true);
  const [gitignoreCount, setGitignoreCount] = useState<number | null>(null);
  const [gitignoreLoading, setGitignoreLoading] = useState(false);
  const [patterns, setPatterns] = useState<string[]>([]);

  // Step 4: preview
  const [previewPaths, setPreviewPaths] = useState<string[] | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState("");

  // Registration
  const [registering, setRegistering] = useState(false);
  const [registerError, setRegisterError] = useState("");

  // --- Step 1: Pick directory ---

  const handlePickDirectory = async () => {
    const selected = await pickDirectory();
    if (selected) {
      setDirPath(selected);
      // Default name to last path segment
      const segments = selected.replace(/[/\\]$/, "").split(/[/\\]/);
      const baseName = segments[segments.length - 1] || selected;
      setName(baseName);
      setStep(2);
    }
  };

  // --- Step 3: Gitignore pattern count ---

  useEffect(() => {
    if (step !== 3 || !importGitignore || !dirPath) {
      setGitignoreCount(null);
      return;
    }
    let cancelled = false;
    setGitignoreLoading(true);
    repoGitignorePatterns(dirPath)
      .then((pats) => {
        if (!cancelled) {
          setGitignoreCount(pats.length);
          setGitignoreLoading(false);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setGitignoreCount(null);
          setGitignoreLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [step, importGitignore, dirPath]);

  // --- Step 4: Preview ---

  useEffect(() => {
    if (step !== 4 || !dirPath) return;
    let cancelled = false;
    setPreviewLoading(true);
    setPreviewError("");
    setPreviewPaths(null);

    repoPreview(dirPath, patterns, importGitignore)
      .then((paths) => {
        if (!cancelled) {
          setPreviewPaths(paths);
          setPreviewLoading(false);
        }
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setPreviewError(
            isCommandError(e) ? e.error : "Preview failed",
          );
          setPreviewLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [step, dirPath, patterns, importGitignore]);

  // --- Register ---

  const handleRegister = async () => {
    setRegistering(true);
    setRegisterError("");
    try {
      const result = await repoAdd(dirPath, name, patterns, importGitignore);
      onComplete(result.id);
    } catch (e: unknown) {
      setRegisterError(
        isCommandError(e) ? e.error : "Registration failed",
      );
      setRegistering(false);
    }
  };

  // --- Navigation helpers ---

  const canGoNext = (): boolean => {
    switch (step) {
      case 1:
        return dirPath.length > 0;
      case 2:
        return name.trim().length > 0;
      case 3:
        return true;
      default:
        return false;
    }
  };

  const goNext = () => {
    if (canGoNext() && step < 4) {
      setStep(step + 1);
    }
  };

  const goBack = () => {
    if (step > 1) {
      setStep(step - 1);
    }
  };

  // --- Step indicator ---

  const stepLabels = ["Directory", "Name", "Patterns", "Preview"];

  return (
    <div>
      {/* Step indicator */}
      <div className="flex items-center gap-1 mb-6">
        {stepLabels.map((label, i) => {
          const num = i + 1;
          const isActive = num === step;
          const isDone = num < step;
          return (
            <div key={label} className="flex items-center gap-1">
              {i > 0 && (
                <div
                  className={cn(
                    "w-8 h-px",
                    isDone ? "bg-accent" : "bg-border",
                  )}
                />
              )}
              <div
                className={cn(
                  "flex items-center gap-1.5 text-sm",
                  isActive
                    ? "text-accent font-medium"
                    : isDone
                      ? "text-success"
                      : "text-text-muted",
                )}
              >
                <span
                  className={cn(
                    "inline-flex items-center justify-center w-5 h-5 rounded-full text-xs",
                    isActive
                      ? "bg-accent text-white"
                      : isDone
                        ? "bg-success text-white"
                        : "bg-border text-text-muted",
                  )}
                >
                  {isDone ? <Check className="h-3 w-3" /> : num}
                </span>
                {label}
              </div>
            </div>
          );
        })}
      </div>

      {/* Step content */}
      <div className="min-h-[200px]">
        {/* Step 1: Pick directory */}
        {step === 1 && (
          <div>
            <p className="text-sm text-text mb-4">
              Choose the directory to register as a repository.
            </p>
            <button
              onClick={handlePickDirectory}
              className={cn(
                "inline-flex items-center gap-2 rounded border border-border px-4 py-2.5 text-sm",
                "bg-surface text-text hover:border-accent",
                "focus:outline-none focus:ring-1 focus:ring-accent",
              )}
            >
              <FolderOpen className="h-4 w-4 text-text-muted" />
              {dirPath ? "Change directory" : "Browse..."}
            </button>
            {dirPath && (
              <p className="mt-3 text-sm text-text font-mono bg-surface rounded border border-border-muted px-3 py-2">
                {dirPath}
              </p>
            )}
          </div>
        )}

        {/* Step 2: Name */}
        {step === 2 && (
          <div>
            <p className="text-sm text-text mb-4">
              Name for this repository. Defaults to the directory name.
            </p>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && canGoNext()) goNext();
              }}
              placeholder="repo name"
              autoFocus
              className={cn(
                "w-full max-w-sm rounded border border-border px-3 py-1.5 text-sm",
                "bg-surface text-text",
                "placeholder:text-text-muted",
                "focus:outline-none focus:ring-1 focus:ring-accent",
              )}
            />
            {name.trim().length === 0 && (
              <p className="mt-2 text-xs text-warning">
                Name cannot be empty.
              </p>
            )}
          </div>
        )}

        {/* Step 3: Patterns */}
        {step === 3 && (
          <div>
            <div className="mb-4">
              <label className="flex items-center gap-2 text-sm text-text cursor-pointer">
                <input
                  type="checkbox"
                  checked={importGitignore}
                  onChange={(e) => setImportGitignore(e.target.checked)}
                  className="rounded border-border text-accent focus:ring-accent"
                />
                Import patterns from .gitignore
              </label>
              {importGitignore && (
                <p className="mt-1.5 ml-6 text-xs text-text-muted">
                  {gitignoreLoading
                    ? "Scanning .gitignore files..."
                    : gitignoreCount !== null
                      ? `Will import ${gitignoreCount} pattern${gitignoreCount === 1 ? "" : "s"} from .gitignore`
                      : ""}
                </p>
              )}
            </div>

            <PatternEditor
              patterns={patterns}
              onChange={setPatterns}
              label="Additional patterns"
            />
          </div>
        )}

        {/* Step 4: Preview and confirm */}
        {step === 4 && (
          <div>
            {previewLoading && (
              <div className="flex items-center gap-2 text-sm text-text-muted">
                <Loader2 className="h-4 w-4 animate-spin" />
                Scanning files...
              </div>
            )}

            {previewError && (
              <p className="text-sm text-danger">{previewError}</p>
            )}

            {previewPaths && !previewLoading && (
              <div>
                <p className="text-sm text-text mb-3">
                  {previewPaths.length} file{previewPaths.length === 1 ? "" : "s"} will be ingested.
                </p>
                <div className="max-h-64 overflow-y-auto rounded border border-border-muted bg-surface">
                  {previewPaths.map((p) => (
                    <div
                      key={p}
                      className="px-3 py-1 text-xs font-mono text-text border-b border-border-muted last:border-b-0"
                    >
                      {p}
                    </div>
                  ))}
                </div>
              </div>
            )}

            {registerError && (
              <p className="text-sm text-danger mt-3">{registerError}</p>
            )}
          </div>
        )}
      </div>

      {/* Navigation bar */}
      <div className="flex items-center justify-between mt-6 pt-4 border-t border-border-muted">
        <div>
          {step === 1 ? (
            <button
              onClick={onCancel}
              className="text-sm text-text-muted hover:text-text"
            >
              Cancel
            </button>
          ) : (
            <button
              onClick={goBack}
              className="inline-flex items-center gap-1 text-sm text-text-muted hover:text-text"
            >
              <ArrowLeft className="h-3.5 w-3.5" />
              Back
            </button>
          )}
        </div>

        <div className="flex items-center gap-3">
          {/* Skip preview shortcut on step 4 */}
          {step === 4 && previewLoading && (
            <button
              onClick={handleRegister}
              disabled={registering}
              className="text-sm text-text-muted hover:text-text"
            >
              Skip preview, register now
            </button>
          )}

          {step < 4 ? (
            <button
              onClick={goNext}
              disabled={!canGoNext()}
              className={cn(
                "inline-flex items-center gap-1 rounded px-4 py-1.5 text-sm font-medium",
                "focus:outline-none focus:ring-1 focus:ring-accent",
                canGoNext()
                  ? "bg-accent text-white hover:opacity-90"
                  : "bg-border text-text-muted cursor-not-allowed",
              )}
            >
              Next
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          ) : (
            <button
              onClick={handleRegister}
              disabled={registering || previewLoading}
              className={cn(
                "inline-flex items-center gap-1 rounded px-4 py-1.5 text-sm font-medium",
                "focus:outline-none focus:ring-1 focus:ring-accent",
                registering || previewLoading
                  ? "bg-border text-text-muted cursor-not-allowed"
                  : "bg-accent text-white hover:opacity-90",
              )}
            >
              {registering ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Registering...
                </>
              ) : (
                <>
                  <Check className="h-3.5 w-3.5" />
                  Register
                </>
              )}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
