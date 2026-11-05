import { useState, type FormEvent } from "react";

export function Members({ members }: { members: Member[] }) {
  const [email, setEmail] = useState<string>("");
  const [saving, setSaving] = useState(false);
  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setSaving(true);
    await api.invite(email);
    setSaving(false);
  }
  if (members.length < 1) return null;
  return (
    <main className="page">
      <h1 className="title">{t("members.title")}</h1>
      <p className="lede">{t("members.lede")}</p>
      <form onSubmit={onSubmit} className="invite">
        <label htmlFor="email">{t("members.email")}</label>
        <input id="email" type="email" className="field" value={email} onChange={(e) => setEmail(e.target.value)} />
        <button type="submit" className="button focus-visible:ring-2" data-primary disabled={saving}>{t("members.invite")}</button>
      </form>
      <ul className="list">
        {members.map((m) => (<li key={m.id}>{m.name}</li>))}
      </ul>
    </main>
  );
}
