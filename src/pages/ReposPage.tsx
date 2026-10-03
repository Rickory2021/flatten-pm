// src/pages/ReposPage.tsx
//
// Repos page: list registered repos, add new ones, view details.
// Uses an internal PageView state machine (no sub-routes).

import { useState } from "react";
import { FolderGit2 } from "lucide-react";
import { RepoList } from "@/components/repos/RepoList";
import { RepoWizard } from "@/components/repos/RepoWizard";
import { RepoDetail } from "@/components/repos/RepoDetail";

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
  // Incrementing key forces RepoList to remount and refetch after mutations
  // (registration, delete). Without this, React reuses the component and
  // the stale list persists.
  const [listKey, setListKey] = useState(0);

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
            key={listKey}
            onAdd={() => setView({ kind: "wizard" })}
            onSelect={(repoId) => setView({ kind: "detail", repoId })}
          />
        )}

        {view.kind === "wizard" && (
          <RepoWizard
            onCancel={() => setView({ kind: "list" })}
            onComplete={(repoId) => {
              setListKey((k) => k + 1);
              setView({ kind: "detail", repoId });
            }}
          />
        )}

        {view.kind === "detail" && (
          <RepoDetail
            repoId={view.repoId}
            onBack={() => setView({ kind: "list" })}
            onDeleted={() => setListKey((k) => k + 1)}
          />
        )}
      </div>
    </div>
  );
}
