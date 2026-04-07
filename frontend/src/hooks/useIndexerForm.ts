import { useState } from "react";
import { useForm, type UseFormReturn } from "react-hook-form";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import * as api from "@/api";
import type { IndexerResponse, TestIndexerResponse } from "@/types/api";

export interface IndexerFormData {
  name: string;
  protocol: "torrent" | "usenet";
  url: string;
  apiPath: string;
  apiKey: string;
  categories: string;
  priority: number;
  enableAutomaticSearch: boolean;
  enableInteractiveSearch: boolean;
  enabled: boolean;
}

export const indexerFormDefaults: IndexerFormData = {
  name: "",
  protocol: "torrent",
  url: "",
  apiPath: "/",
  apiKey: "",
  categories: "7020, 3030",
  priority: 1,
  enableAutomaticSearch: true,
  enableInteractiveSearch: true,
  enabled: true,
};

export function parseCategories(s: string): number[] {
  if (!s.trim()) return [];
  return s
    .split(",")
    .map((c) => c.trim())
    .filter((c) => c.length > 0 && /^\d+$/.test(c))
    .map((c) => Number(c));
}

export function useIndexerForm(editing: IndexerResponse | null) {
  const [testResult, setTestResult] = useState<TestIndexerResponse | null>(null);

  const form: UseFormReturn<IndexerFormData> = useForm<IndexerFormData>({
    values: editing
      ? {
          name: editing.name,
          protocol: editing.protocol,
          url: editing.url,
          apiPath: editing.apiPath,
          apiKey: "",
          categories: editing.categories.join(", "),
          priority: editing.priority,
          enableAutomaticSearch: editing.enableAutomaticSearch,
          enableInteractiveSearch: editing.enableInteractiveSearch,
          enabled: editing.enabled,
        }
      : undefined,
    defaultValues: indexerFormDefaults,
  });

  const handleTestResult = (result: TestIndexerResponse) => {
    setTestResult(result);
    if (result.ok) toast.success("Connection successful");
    else toast.error(result.error ?? "Test failed");
  };

  const handleTestError = (e: Error) => {
    setTestResult({ ok: false, supportsBookSearch: false, error: e.message });
    toast.error(e.message);
  };

  const testIndexer = useMutation({
    mutationFn: api.testIndexer,
    onSuccess: handleTestResult,
    onError: handleTestError,
  });

  const testSaved = useMutation({
    mutationFn: api.testSavedIndexer,
    onSuccess: handleTestResult,
    onError: handleTestError,
  });

  const runTest = () => {
    const vals = form.getValues();
    setTestResult(null);
    if (editing && !vals.apiKey) {
      testSaved.mutate(editing.id);
    } else {
      if (!vals.url) {
        toast.error("URL is required to test");
        return;
      }
      testIndexer.mutate({
        url: vals.url,
        apiPath: vals.apiPath || "/",
        apiKey: vals.apiKey || null,
      });
    }
  };

  const isTesting = testIndexer.isPending || testSaved.isPending;

  const resetTest = () => setTestResult(null);

  return { form, testResult, isTesting, runTest, resetTest };
}
