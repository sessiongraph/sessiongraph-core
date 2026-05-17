// Dashboard — primary view. See spec section 6.3.
// Stub: real stats wiring lands in Week 1 Task 10.
export default function Dashboard() {
  return (
    <main className="mx-auto max-w-5xl px-8 py-10">
      <header className="flex items-center justify-between border-b border-border pb-4">
        <h1 className="text-xl font-semibold">SessionGraph</h1>
        <span className="text-sm text-text-secondary">Proxy Active</span>
      </header>
      <section className="mt-10">
        <p className="text-text-secondary">
          Dashboard scaffold — live stats will appear here.
        </p>
      </section>
    </main>
  );
}
