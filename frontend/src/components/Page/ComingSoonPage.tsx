import { Construction } from "lucide-react";
import { PageContent } from "./PageContent";
import { EmptyState } from "./EmptyState";

export function ComingSoonPage({ title }: { title: string }) {
  return (
    <PageContent>
      <EmptyState
        icon={<Construction size={48} />}
        title={title}
        description="This feature is coming in a future release."
      />
    </PageContent>
  );
}
