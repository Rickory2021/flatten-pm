// src/pages/RecipesPage.tsx
import { FileCode2 } from "lucide-react";

export function RecipesPage() {
  return (
    <div className="p-6">
      <div className="flex items-center gap-2">
        <FileCode2 className="h-5 w-5 text-accent" />
        <h1 className="text-xl font-semibold text-text">Recipes</h1>
      </div>
      <p className="text-text-muted mt-1">
        Build recipes for export pipelines.
      </p>
    </div>
  );
}
