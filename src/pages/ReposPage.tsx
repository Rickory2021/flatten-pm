// src/pages/ReposPage.tsx
import { FolderGit2 } from "lucide-react";

export function ReposPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <FolderGit2 className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Repos</h1>
      </div>
      <p className="text-text-muted mt-1">
        Registered repositories and ingest configuration.
      </p>
    </div>
  );
}
