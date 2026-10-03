// src/pages/TemplatesPage.tsx
import { FileText } from "lucide-react";

export function TemplatesPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <FileText className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Templates</h1>
      </div>
      <p className="text-text-muted mt-1">
        Enrichment and file templates for export.
      </p>
    </div>
  );
}
