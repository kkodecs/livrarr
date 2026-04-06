import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm, Controller } from "react-hook-form";
import { toast } from "sonner";
import {
  Users,
  Plus,
  Trash2,
  Pencil,
  KeyRound,
  ShieldCheck,
  User,
} from "lucide-react";
import { useAuthStore } from "@/stores/auth";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import { formatRelativeDate, formatAbsoluteDate } from "@/utils/format";
import { useUIStore } from "@/stores/ui";
import { useSort } from "@/hooks/useSort";
import { SortHeader } from "@/components/Page/SortHeader";
import type { UserResponse, UserRole } from "@/types/api";
import * as api from "@/api";

type UserSortField = "username" | "role" | "createdAt";

// ── Form Types ──

interface CreateUserForm {
  username: string;
  password: string;
  confirmPassword: string;
  role: UserRole;
}

interface EditUserForm {
  username: string;
  password: string;
  confirmPassword: string;
  role: UserRole;
}

// ── Main Page ──

export default function UsersPage() {
  const currentUser = useAuthStore((s) => s.user);
  const qc = useQueryClient();
  const relativeDates = useUIStore((s) => s.relativeDates);

  const usersQ = useQuery({ queryKey: ["users"], queryFn: api.listUsers });

  const createUser = useMutation({
    mutationFn: api.createUser,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["users"] });
      toast.success("User created");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const updateUser = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: number;
      data: {
        username?: string | null;
        password?: string | null;
        role?: UserRole | null;
      };
    }) => api.updateUser(id, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["users"] });
      toast.success("User updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const deleteUser = useMutation({
    mutationFn: api.deleteUser,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["users"] });
      toast.success("User deleted");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const regenApiKey = useMutation({
    mutationFn: api.regenerateUserApiKey,
    onSuccess: (data) => {
      toast.success(`API key regenerated: ${data.apiKey}`, { duration: 10000 });
    },
    onError: (e: Error) => toast.error(e.message),
  });

  // Modal state
  const [createModal, setCreateModal] = useState(false);
  const [editTarget, setEditTarget] = useState<UserResponse | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<UserResponse | null>(null);
  const [regenTarget, setRegenTarget] = useState<UserResponse | null>(null);

  const sorting = useSort<UserSortField>("username");

  if (usersQ.isLoading) return <PageLoading />;
  if (usersQ.error)
    return (
      <ErrorState
        error={usersQ.error as Error}
        onRetry={() => usersQ.refetch()}
      />
    );

  const users = sorting.sort(usersQ.data ?? [], (item, field) => {
    switch (field) {
      case "username": return item.username;
      case "role": return item.role;
      case "createdAt": return item.createdAt;
    }
  });
  const adminCount = users.filter((u) => u.role === "admin").length;

  const canDeleteUser = (u: UserResponse) => {
    if (u.id === currentUser?.id) return false; // can't delete self
    if (u.role === "admin" && adminCount <= 1) return false; // can't delete last admin
    return true;
  };

  const canDemoteUser = (u: UserResponse) => {
    if (u.role === "admin" && adminCount <= 1) return false; // can't demote last admin
    return true;
  };

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Users</h1>
        <button
          onClick={() => setCreateModal(true)}
          className="inline-flex items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-brand-hover"
        >
          <Plus size={14} /> Add User
        </button>
      </PageToolbar>

      <PageContent>
        {users.length > 0 ? (
          <div className="overflow-x-auto rounded border border-border">
            <table className="w-full text-sm">
              <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                <tr>
                  <SortHeader field="username" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Username</SortHeader>
                  <SortHeader field="role" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Role</SortHeader>
                  <SortHeader field="createdAt" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Created</SortHeader>
                  <th className="px-4 py-2 w-32" />
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {users.map((u) => (
                  <tr key={u.id} className="text-zinc-200">
                    <td className="px-4 py-2 font-medium">
                      {u.username}
                      {u.id === currentUser?.id && (
                        <span className="ml-2 text-xs text-muted">(you)</span>
                      )}
                    </td>
                    <td className="px-4 py-2">
                      <RoleBadge role={u.role} />
                    </td>
                    <td className="px-4 py-2 text-xs text-muted">
                      {relativeDates
                        ? formatRelativeDate(u.createdAt)
                        : formatAbsoluteDate(u.createdAt)}
                    </td>
                    <td className="px-4 py-2">
                      <div className="flex items-center gap-2">
                        <button
                          onClick={() => setEditTarget(u)}
                          className="text-muted hover:text-zinc-100"
                          title="Edit user"
                        >
                          <Pencil size={14} />
                        </button>
                        <button
                          onClick={() => setRegenTarget(u)}
                          className="text-muted hover:text-zinc-100"
                          title="Regenerate API key"
                        >
                          <KeyRound size={14} />
                        </button>
                        {canDeleteUser(u) && (
                          <button
                            onClick={() => setDeleteTarget(u)}
                            className="text-muted hover:text-red-400"
                            title="Delete user"
                          >
                            <Trash2 size={14} />
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <EmptyState icon={<Users size={28} />} title="No users" />
        )}
      </PageContent>

      {/* ── Create User Modal ── */}
      <CreateUserModal
        open={createModal}
        onClose={() => setCreateModal(false)}
        onSubmit={async (data) => {
          await createUser.mutateAsync({
            username: data.username,
            password: data.password,
            role: data.role,
          });
        }}
      />

      {/* ── Edit User Modal ── */}
      {editTarget && (
        <EditUserModal
          open={editTarget !== null}
          user={editTarget}
          canDemote={canDemoteUser(editTarget)}
          onClose={() => setEditTarget(null)}
          onSubmit={async (data) => {
            const req: {
              username?: string | null;
              password?: string | null;
              role?: UserRole | null;
            } = {};
            if (data.username !== editTarget.username)
              req.username = data.username;
            if (data.password) req.password = data.password;
            if (data.role !== editTarget.role) req.role = data.role;
            await updateUser.mutateAsync({ id: editTarget.id, data: req });
          }}
        />
      )}

      {/* ── Delete User Confirm ── */}
      <ConfirmModal
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        title="Delete User"
        description={`Permanently delete user "${deleteTarget?.username}"? This cannot be undone.`}
        confirmLabel="Delete"
        typeConfirm={deleteTarget?.username}
        onConfirm={() => {
          if (deleteTarget) return deleteUser.mutateAsync(deleteTarget.id);
        }}
      />

      {/* ── Regenerate API Key Confirm ── */}
      <ConfirmModal
        open={regenTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRegenTarget(null);
        }}
        title="Regenerate API Key"
        description={`Generate a new API key for "${regenTarget?.username}"? The current key will be invalidated.`}
        confirmLabel="Regenerate"
        variant="default"
        onConfirm={async () => {
          if (regenTarget) {
            await regenApiKey.mutateAsync(regenTarget.id);
          }
        }}
      />
    </>
  );
}

// ── Role Badge ──

function RoleBadge({ role }: { role: UserRole }) {
  if (role === "admin") {
    return (
      <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/20 px-2 py-0.5 text-xs font-medium text-amber-400">
        <ShieldCheck size={12} /> Admin
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1 rounded-full bg-zinc-600/20 px-2 py-0.5 text-xs font-medium text-zinc-400">
      <User size={12} /> User
    </span>
  );
}

// ── Create User Modal ──

function CreateUserModal({
  open,
  onClose,
  onSubmit,
}: {
  open: boolean;
  onClose: () => void;
  onSubmit: (data: CreateUserForm) => Promise<void>;
}) {
  const {
    register,
    handleSubmit,
    watch,
    control,
    formState: { errors, isSubmitting },
  } = useForm<CreateUserForm>({
    defaultValues: {
      username: "",
      password: "",
      confirmPassword: "",
      role: "user",
    },
  });

  const password = watch("password");

  return (
    <FormModal
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Add User"
    >
      <form
        onSubmit={handleSubmit(async (data) => {
          await onSubmit(data);
          onClose();
        })}
        className="space-y-4"
      >
        <div>
          <label className="block text-xs text-muted mb-1">Username</label>
          <input
            {...register("username", {
              required: "Username is required",
              minLength: { value: 3, message: "Min 3 characters" },
            })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          {errors.username && (
            <p className="mt-1 text-xs text-red-400">
              {errors.username.message}
            </p>
          )}
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">Password</label>
          <input
            type="password"
            {...register("password", {
              required: "Password is required",
              minLength: { value: 8, message: "Min 8 characters" },
            })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          {errors.password && (
            <p className="mt-1 text-xs text-red-400">
              {errors.password.message}
            </p>
          )}
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">
            Confirm Password
          </label>
          <input
            type="password"
            {...register("confirmPassword", {
              validate: (v) => v === password || "Passwords do not match",
            })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          {errors.confirmPassword && (
            <p className="mt-1 text-xs text-red-400">
              {errors.confirmPassword.message}
            </p>
          )}
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">Role</label>
          <Controller
            name="role"
            control={control}
            render={({ field }) => (
              <select
                value={field.value}
                onChange={field.onChange}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              >
                <option value="user">User</option>
                <option value="admin">Admin</option>
              </select>
            )}
          />
        </div>

        <div className="flex justify-end gap-3 pt-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={isSubmitting}
            className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {isSubmitting ? "Creating..." : "Create User"}
          </button>
        </div>
      </form>
    </FormModal>
  );
}

// ── Edit User Modal ──

function EditUserModal({
  open,
  user,
  canDemote,
  onClose,
  onSubmit,
}: {
  open: boolean;
  user: UserResponse;
  canDemote: boolean;
  onClose: () => void;
  onSubmit: (data: EditUserForm) => Promise<void>;
}) {
  const {
    register,
    handleSubmit,
    watch,
    control,
    formState: { errors, isSubmitting },
  } = useForm<EditUserForm>({
    values: {
      username: user.username,
      password: "",
      confirmPassword: "",
      role: user.role,
    },
  });

  const password = watch("password");

  return (
    <FormModal
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title="Edit User"
    >
      <form
        onSubmit={handleSubmit(async (data) => {
          await onSubmit(data);
          onClose();
        })}
        className="space-y-4"
      >
        <div>
          <label className="block text-xs text-muted mb-1">Username</label>
          <input
            {...register("username", {
              required: "Username is required",
              minLength: { value: 3, message: "Min 3 characters" },
            })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          {errors.username && (
            <p className="mt-1 text-xs text-red-400">
              {errors.username.message}
            </p>
          )}
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">Password</label>
          <input
            type="password"
            {...register("password", {
              minLength: { value: 8, message: "Min 8 characters" },
            })}
            placeholder="Leave blank to keep current"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          {errors.password && (
            <p className="mt-1 text-xs text-red-400">
              {errors.password.message}
            </p>
          )}
        </div>

        {password && (
          <div>
            <label className="block text-xs text-muted mb-1">
              Confirm Password
            </label>
            <input
              type="password"
              {...register("confirmPassword", {
                validate: (v) =>
                  !password || v === password || "Passwords do not match",
              })}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
            {errors.confirmPassword && (
              <p className="mt-1 text-xs text-red-400">
                {errors.confirmPassword.message}
              </p>
            )}
          </div>
        )}

        <div>
          <label className="block text-xs text-muted mb-1">Role</label>
          <Controller
            name="role"
            control={control}
            render={({ field }) => (
              <select
                value={field.value}
                onChange={(e) => {
                  if (e.target.value === "user" && !canDemote) {
                    toast.error("Cannot demote the last admin");
                    return;
                  }
                  field.onChange(e);
                }}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              >
                <option value="user">User</option>
                <option value="admin">Admin</option>
              </select>
            )}
          />
          {!canDemote && user.role === "admin" && (
            <p className="mt-1 text-xs text-amber-400">
              This is the only admin and cannot be demoted.
            </p>
          )}
        </div>

        <div className="flex justify-end gap-3 pt-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={isSubmitting}
            className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {isSubmitting ? "Saving..." : "Save Changes"}
          </button>
        </div>
      </form>
    </FormModal>
  );
}
