import { FileQuestionMark } from "lucide-react";
import { PageContent } from "./PageContent";
import { EmptyState } from "./EmptyState";

export function NotFoundPage() {
  return (
    <PageContent>
      <EmptyState
        icon={<FileQuestionMark size={48} />}
        title="Page Not Found"
        description="This page does not exist."
      />
    </PageContent>
  );
}
