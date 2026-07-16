import * as Tabs from "@radix-ui/react-tabs";

export function TabTrigger({
  value,
  children,
}: {
  value: string;
  children: React.ReactNode;
}) {
  return (
    <Tabs.Trigger
      value={value}
      className="border-b-2 border-transparent px-4 py-2 text-sm font-medium text-muted hover:text-zinc-100 data-[state=active]:border-brand data-[state=active]:text-zinc-100"
    >
      {children}
    </Tabs.Trigger>
  );
}
