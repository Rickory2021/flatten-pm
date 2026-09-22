// src/pages/ReposPage.tsx
//
// Repos page: list registered repos, add new ones, view details.
// Uses an internal PageView state machine (no sub-routes).

import { useState } from "react";
import { FolderGit2 } from "lucide-react";
import { RepoList } from "@/components/repos/RepoList";

// ---------------------------------------------------------------------------
// Page view state
// ---------------------------------------------------------------------------

type PageView =
  | { kind: "list" }
  | { kind: "wizard" }
  | { kind: "detail"; repoId: number };

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

export function ReposPage() {
  const [view, setView] = useState<PageView>({ kind: "list" });

  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <FolderGit2 className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Repos</h1>
      </div>
      <p className="text-text-muted mt-1">
        Registered repositories and ingest configuration.
      </p>

      <div className="mt-6">
        {view.kind === "list" && (
          <RepoList
            onAdd={() => setView({ kind: "wizard" })}
            onSelect={(repoId) => setView({ kind: "detail", repoId })}
          />
        )}

        {view.kind === "wizard" && (
          <div className="text-text-muted text-sm">
            {/* Stub, replaced in chunk 5 */}
            <p>Registration wizard placeholder.</p>
            <button
              onClick={() => setView({ kind: "list" })}
              className="mt-2 text-accent hover:underline text-sm"
            >
              Back to list
            </button>
          </div>
        )}

        {view.kind === "detail" && (
          <div className="text-text-muted text-sm">
            {/* Stub, replaced in chunk 6 */}
            <p>Detail view placeholder for repo {view.repoId}.</p>
            <button
              onClick={() => setView({ kind: "list" })}
              className="mt-2 text-accent hover:underline text-sm"
            >
              Back to list
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
