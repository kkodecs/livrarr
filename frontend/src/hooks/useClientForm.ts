import { useState } from "react";
import { useForm, type UseFormReturn } from "react-hook-form";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import * as api from "@/api";
import type {
  DownloadClientResponse,
  CreateDownloadClientRequest,
  DownloadClientImplementation,
} from "@/types/api";

export interface ClientFormData {
  implementation: DownloadClientImplementation;
  name: string;
  host: string;
  port: number;
  useSsl: boolean;
  skipSslValidation: boolean;
  urlBase: string;
  username: string;
  password: string;
  apiKey: string;
  category: string;
  enabled: boolean;
  isDefaultForProtocol: boolean;
}

export const clientFormDefaults: ClientFormData = {
  implementation: "qBittorrent",
  name: "",
  host: "localhost",
  port: 8080,
  useSsl: false,
  skipSslValidation: false,
  urlBase: "",
  username: "",
  password: "",
  apiKey: "",
  category: "livrarr",
  enabled: true,
  isDefaultForProtocol: false,
};

export function toClientRequest(data: ClientFormData): CreateDownloadClientRequest {
  return {
    name: data.name,
    implementation: data.implementation,
    host: data.host,
    port: data.port,
    useSsl: data.useSsl,
    skipSslValidation: data.skipSslValidation,
    urlBase: data.urlBase || null,
    username: data.implementation === "qBittorrent" ? data.username || null : null,
    password: data.implementation === "qBittorrent" ? data.password || null : null,
    category: data.category,
    enabled: data.enabled,
    apiKey: data.implementation === "sabnzbd" ? data.apiKey || null : null,
    isDefaultForProtocol: data.isDefaultForProtocol,
  };
}

export function useClientForm(editing: DownloadClientResponse | null) {
  const [testResult, setTestResult] = useState<"success" | "fail" | null>(null);

  const form: UseFormReturn<ClientFormData> = useForm<ClientFormData>({
    values: editing
      ? {
          implementation: editing.implementation,
          name: editing.name,
          host: editing.host,
          port: editing.port,
          useSsl: editing.useSsl,
          skipSslValidation: editing.skipSslValidation,
          urlBase: editing.urlBase ?? "",
          username: editing.username ?? "",
          password: "",
          apiKey: "",
          category: editing.category,
          enabled: editing.enabled,
          isDefaultForProtocol: editing.isDefaultForProtocol,
        }
      : undefined,
    defaultValues: clientFormDefaults,
  });

  const testClient = useMutation({
    mutationFn: api.testDownloadClient,
    onSuccess: () => {
      setTestResult("success");
      toast.success("Connection successful");
    },
    onError: (e: Error) => {
      setTestResult("fail");
      toast.error(e.message);
    },
  });

  const testSaved = useMutation({
    mutationFn: api.testSavedDownloadClient,
    onSuccess: () => {
      setTestResult("success");
      toast.success("Connection successful");
    },
    onError: (e: Error) => {
      setTestResult("fail");
      toast.error(e.message);
    },
  });

  const runTest = () => {
    setTestResult(null);
    // When editing and no credentials have been changed, test the saved client
    // directly so the server reads credentials from DB.
    if (editing) {
      const vals = form.getValues();
      const hasNewCreds =
        (editing.implementation === "sabnzbd" && vals.apiKey) ||
        (editing.implementation === "qBittorrent" && vals.password);
      if (!hasNewCreds) {
        testSaved.mutate(editing.id);
        return;
      }
    }
    testClient.mutate(toClientRequest(form.getValues()));
  };

  const isTesting = testClient.isPending || testSaved.isPending;
  const resetTest = () => setTestResult(null);

  return { form, testResult, isTesting, runTest, resetTest };
}
