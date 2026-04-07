import { Navigate, useLocation } from "react-router";
import { useAuthStore } from "@/stores/auth";
import type { ReactNode } from "react";

export function AuthGuard({ children }: { children: ReactNode }) {
  const status = useAuthStore((s) => s.status);
  const location = useLocation();

  if (status === "loading") {
    return null;
  }

  if (status === "setup_required") {
    return <Navigate to="/setup" replace />;
  }

  if (status === "unauthenticated") {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }

  return <>{children}</>;
}

export function AdminGuard({ children }: { children: ReactNode }) {
  const isAdmin = useAuthStore((s) => s.isAdmin);

  if (!isAdmin) {
    return <Navigate to="/" replace />;
  }

  return <>{children}</>;
}

export function GuestGuard({ children }: { children: ReactNode }) {
  const status = useAuthStore((s) => s.status);

  if (status === "loading") {
    return null;
  }

  if (status === "authenticated") {
    return <Navigate to="/" replace />;
  }

  return <>{children}</>;
}
