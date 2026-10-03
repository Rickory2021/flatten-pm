// src/pages/WatchPage.tsx
import { Eye } from "lucide-react";

export function WatchPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <Eye className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Watch</h1>
      </div>
      <p className="text-text-muted mt-1">
        Watch session management, status, and flag review.
      </p>
    </div>
  );
}
