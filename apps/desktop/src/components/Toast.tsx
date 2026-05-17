import { useNotificationsStore } from "../stores/notifications";

export default function ToastContainer() {
  const { notifications, removeNotification } = useNotificationsStore();

  if (notifications.length === 0) return null;

  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50 flex flex-col gap-2">
      {notifications.map((n) => (
        <div
          key={n.id}
          className={`pointer-events-auto flex max-w-xs items-start gap-2 rounded-lg border px-4 py-3 text-sm shadow-lg ${
            n.type === "error"
              ? "border-red-500/30 bg-red-500/10 text-red-300"
              : n.type === "success"
                ? "border-success/30 bg-success/10 text-success"
                : "border-accent/30 bg-accent/10 text-accent"
          }`}
        >
          <span className="flex-1">{n.message}</span>
          <button
            onClick={() => removeNotification(n.id)}
            className="text-current/50 hover:text-current shrink-0"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
