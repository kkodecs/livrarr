import { useState } from "react";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import { useAuthStore } from "@/stores/auth";
import { updateProfile, regenerateApiKey } from "@/api";
import { PageContent } from "@/components/Page/PageContent";

interface ProfileForm {
  username: string;
  newPassword: string;
  confirmPassword: string;
}

export default function ProfilePage() {
  const user = useAuthStore((s) => s.user);
  const refreshUser = useAuthStore((s) => s.refreshUser);
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [regenerating, setRegenerating] = useState(false);

  const {
    register,
    handleSubmit,
    watch,
    reset,
    formState: { isSubmitting, errors },
  } = useForm<ProfileForm>({
    defaultValues: { username: user?.username ?? "" },
  });

  const onSubmit = async (data: ProfileForm) => {
    try {
      await updateProfile({
        username: data.username,
        password: data.newPassword || null,
      });
      await refreshUser();
      toast.success("Profile updated");
      reset({ ...data, newPassword: "", confirmPassword: "" });
    } catch (e: any) {
      toast.error(e?.message ?? "Failed to update profile");
    }
  };

  const handleRegenerate = async () => {
    setRegenerating(true);
    try {
      const result = await regenerateApiKey();
      setApiKey(result.apiKey);
      toast.success("API key regenerated");
    } catch (e: any) {
      toast.error(e?.message ?? "Failed to regenerate API key");
    } finally {
      setRegenerating(false);
    }
  };

  return (
    <PageContent>
      <div className="max-w-lg space-y-8">
        {/* Edit Profile */}
        <section className="space-y-4">
          <h2 className="text-lg font-semibold text-zinc-100">Account</h2>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Username
              </label>
              <input
                {...register("username", { required: "Required" })}
                className="input-field"
              />
              {errors.username && (
                <p className="mt-0.5 text-xs text-red-400">
                  {errors.username.message}
                </p>
              )}
            </div>

            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                New Password
              </label>
              <input
                {...register("newPassword", {
                  minLength: { value: 8, message: "Min 8 characters" },
                })}
                type="password"
                className="input-field"
                placeholder="Leave blank to keep current"
              />
              {errors.newPassword && (
                <p className="mt-0.5 text-xs text-red-400">
                  {errors.newPassword.message}
                </p>
              )}
            </div>

            <div>
              <label className="mb-1 block text-sm font-medium text-zinc-300">
                Confirm New Password
              </label>
              <input
                {...register("confirmPassword", {
                  validate: (v) => {
                    const pw = watch("newPassword");
                    return !pw || v === pw || "Passwords don't match";
                  },
                })}
                type="password"
                className="input-field"
              />
              {errors.confirmPassword && (
                <p className="mt-0.5 text-xs text-red-400">
                  {errors.confirmPassword.message}
                </p>
              )}
            </div>

            <button
              type="submit"
              disabled={isSubmitting}
              className="btn-primary"
            >
              {isSubmitting ? "Saving..." : "Save Changes"}
            </button>
          </form>
        </section>

        {/* API Key */}
        <section className="space-y-4">
          <h2 className="text-lg font-semibold text-zinc-100">API Key</h2>
          {apiKey ? (
            <>
              <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-3">
                <input
                  readOnly
                  value={apiKey}
                  className="input-field flex-1 font-mono text-sm text-amber-400"
                  onClick={(e) => {
                    (e.target as HTMLInputElement).select();
                    navigator.clipboard.writeText(apiKey);
                    toast.success("Copied to clipboard");
                  }}
                />
                <button
                  onClick={handleRegenerate}
                  disabled={regenerating}
                  className="btn-secondary whitespace-nowrap"
                >
                  {regenerating ? "Regenerating..." : "Regenerate"}
                </button>
              </div>
              <p className="text-xs text-zinc-500">
                Click the key to copy. This is the only time it will be shown.
              </p>
            </>
          ) : (
            <div className="flex flex-col sm:flex-row items-start sm:items-center gap-3">
              <p className="flex-1 text-sm text-zinc-400">
                Your API key is stored securely and cannot be displayed. Regenerate to get a new one.
              </p>
              <button
                onClick={handleRegenerate}
                disabled={regenerating}
                className="btn-secondary whitespace-nowrap"
              >
                {regenerating ? "Regenerating..." : "Regenerate Key"}
              </button>
            </div>
          )}
        </section>
      </div>
    </PageContent>
  );
}
