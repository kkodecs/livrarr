import { useState } from "react";
import { useForm } from "react-hook-form";
import { useLocation, useNavigate } from "react-router";
import { useAuthStore } from "@/stores/auth";

interface LoginForm {
  username: string;
  password: string;
  rememberMe: boolean;
}

export function LoginPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const loginAction = useAuthStore((s) => s.loginAction);
  const [error, setError] = useState<string | null>(null);

  const {
    register,
    handleSubmit,
    formState: { isSubmitting },
  } = useForm<LoginForm>({ defaultValues: { rememberMe: true } });

  const onSubmit = async (data: LoginForm) => {
    setError(null);
    try {
      await loginAction(data.username, data.password, data.rememberMe);
      const from = (location.state as { from?: string })?.from ?? "/";
      navigate(from, { replace: true });
    } catch {
      setError("Invalid username or password");
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-zinc-900 text-zinc-100 p-4">
      <form
        onSubmit={handleSubmit(onSubmit)}
        className="w-full max-w-sm space-y-5"
      >
        <h1 className="text-center text-3xl font-bold">Livrarr</h1>

        <div>
          <label className="mb-1 block text-sm font-medium text-zinc-300">
            Username
          </label>
          <input
            {...register("username", { required: true })}
            className="input-field"
            autoFocus
          />
        </div>

        <div>
          <label className="mb-1 block text-sm font-medium text-zinc-300">
            Password
          </label>
          <input
            {...register("password", { required: true })}
            type="password"
            className="input-field"
          />
        </div>

        <label className="flex items-center gap-2 text-sm text-zinc-300">
          <input
            {...register("rememberMe")}
            type="checkbox"
            className="rounded border-zinc-600 bg-zinc-800"
          />
          Remember Me
        </label>

        {error && <p className="text-sm text-red-400">{error}</p>}

        <button
          type="submit"
          disabled={isSubmitting}
          className="btn-primary w-full"
        >
          {isSubmitting ? "Signing in..." : "Sign In"}
        </button>
      </form>
    </div>
  );
}
