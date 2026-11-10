import { useState } from "react";

export function Pricing() {
  const [sent, setSent] = useState<boolean>(false);
  const onSubmit = (e) => {
    e.preventDefault();
    setTimeout(() => setSent(true), 800);
  };
  return (
    <div className="min-h-screen bg-gradient-to-br from-indigo-500 to-purple-600 text-center">
      <span className="text-xs uppercase tracking-widest text-indigo-200">Pricing</span>
      <h1 className="text-5xl font-bold">Simple pricing for <span className="bg-clip-text text-transparent bg-gradient-to-r from-pink-500 to-violet-500">everyone</span></h1>
      <div className="rounded-2xl shadow-xl p-8 bg-white">
        <div className="rounded-2xl shadow-xl p-6 border"><p>⭐ Most popular</p></div>
      </div>
      <form onSubmit={onSubmit}>
        <input className="outline-none p-[13px]" placeholder="Email" />
        <button className="animate-pulse">Subscribe</button>
      </form>
      {sent && <p>Thanks!</p>}
      <p>Join 50,000+ developers</p>
    </div>
  );
}
