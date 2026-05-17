import { create } from "zustand";

export type Notification = {
  id: string;
  message: string;
  type: "error" | "info" | "success";
  timestamp: number;
};

type NotificationsState = {
  notifications: Notification[];
  addNotification: (message: string, type?: Notification["type"]) => void;
  removeNotification: (id: string) => void;
};

let counter = 0;

export const useNotificationsStore = create<NotificationsState>((set) => ({
  notifications: [],
  addNotification: (message, type = "error") => {
    const id = `notif-${++counter}`;
    set((s) => ({
      notifications: [
        ...s.notifications,
        { id, message, type, timestamp: Date.now() },
      ],
    }));
    setTimeout(() => {
      set((s) => ({
        notifications: s.notifications.filter((n) => n.id !== id),
      }));
    }, 5000);
  },
  removeNotification: (id) => {
    set((s) => ({
      notifications: s.notifications.filter((n) => n.id !== id),
    }));
  },
}));
