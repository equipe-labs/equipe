import fs from 'fs';
const head=(t)=>`<!doctype html><html lang="en"><head><meta charset="utf-8"><title>${t}</title><link rel="stylesheet" href="metrics.css"></head><body>`;
const side=(on)=>`<aside class="side"><div style="padding:4px 12px 12px;font-weight:700;font-size:15px">equipe<span style="color:var(--eq-color-dot)">.</span></div>
<h6>Projects</h6><div class="nav"><i></i>equipe-site</div><div class="nav"><i></i>equipe-app</div><div class="nav"><i></i>equipe-desktop</div>
<h6>Views</h6><div class="nav">Conversation</div><div class="nav">Tickets</div><div class="nav ${on?'on':''}">Metrics</div><div class="nav">Settings</div></aside>`;
const sparkBars=(vals,h=64,w=420,cur=true)=>{const m=Math.max(...vals);const bw=w/vals.length;return `<svg width="100%" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" style="height:${h}px">${vals.map((v,i)=>{const bh=Math.max(2,v/m*(h-4));return `<rect class="${i===vals.length-1&&cur?'fa':'fb'}" x="${i*bw+3}" y="${h-bh}" width="${bw-6}" height="${bh}" rx="3"/>`}).join('')}</svg>`};
const line=(a,b,w=300,h=56)=>{const m=Math.max(...a,...b);const pts=(v)=>v.map((x,i)=>`${(i/(v.length-1)*(w-4)+2).toFixed(1)},${(h-4-x/m*(h-10)).toFixed(1)}`).join(' ');return `<svg width="100%" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" style="height:${h}px"><polyline class="p" fill="none" stroke-width="2" stroke-dasharray="4 3" points="${pts(b)}"/><polyline class="a" fill="none" stroke-width="2.5" stroke-linejoin="round" points="${pts(a)}"/></svg>`};
const bars=(rows)=>rows.map(([n,p,v])=>`<div class="bar"><span>${n}</span><span class="t"><u style="width:${p}%"></u></span><span>${v}</span></div>`).join('');
const d=(k,t)=>`<span class="delta ${k}">${k==='up'?'▲':k==='dn'?'▼':'–'} ${t}</span>`;
const stat=(title,src,num,unit,delta,body='')=>`<div class="card"><h3>${title}<span class="src">${src}</span></h3><div><span class="big m">${num}</span><span class="unit">${unit}</span></div><div>${delta}<span class="note"> vs. previous period</span></div>${body}</div>`;
const bar=`<div class="head"><div><h1>Metrics · equipe-site</h1><div class="sub">Oct 27 – Nov 2 · previous period Oct 20 – Oct 26</div></div><div style="display:flex;gap:12px;align-items:center"><div class="seg"><b class="on">7 days</b><b>30 days</b></div><span class="btn">Refresh</span><span class="btn">Manage connections</span></div></div>`;

const svc=(ic,n,desc,state)=>`<div style="display:flex;gap:12px;align-items:center;padding:12px 0;border-top:1px solid var(--eq-color-line)"><div class="ico">${ic}</div><div style="flex:1"><div style="font-weight:600;font-size:14px">${n}</div><div class="note">${desc}</div></div>${state}</div>`;
const connect=head('Connect')+`<div class="win">${side(true)}<main class="main">
<div class="head"><div><h1>Connect metrics</h1><div class="sub">Connect a service and see its numbers right inside equipe. It only reads; it changes nothing.</div></div></div>
<div class="grid g3" style="align-items:start">
<div class="card"><div class="tag">1 · Pick services</div>
${svc('PH','PostHog','Visits, languages, referrers, click locations','<span class="btn p">Connect</span>')}
${svc('GH','GitHub','Repository traffic, stars','<span class="btn">Connect</span>')}
${svc('AS','App Store Connect','App downloads','<span class="btn">Connect</span>')}
<div class="note">You don't need to connect all three. Only what you connect shows on the dashboard.</div></div>
<div class="card"><div class="tag">2 · Add a key · PostHog</div>
<div class="note" style="color:var(--eq-color-text)">In PostHog → Settings → Personal API keys, make a <b>read-only</b> key and paste it here.</div>
<div style="display:flex;flex-direction:column;gap:4px"><label class="l">Personal API key</label><div class="field f">phx_••••••••••••••••<span style="margin-left:auto;width:1px;height:16px;background:var(--eq-color-text)"></span></div></div>
<div style="display:flex;flex-direction:column;gap:4px"><label class="l">Project ID</label><div class="field">700951</div><div class="note">The number after /project/ in your PostHog address.</div></div>
<div style="display:flex;gap:8px"><span class="btn p">Check and connect</span><span class="btn g">Back</span></div>
<div class="note">🔒 The key is kept on this Mac only. It never goes to equipe's servers or your phone.</div></div>
<div class="card"><div class="tag">3 · Connected</div>
<div style="display:flex;gap:10px;align-items:center"><span class="badge ok">✓ Connected</span><b style="font-size:15px">PostHog</b></div>
<div class="note" style="color:var(--eq-color-text)">What can be read from project 700951</div>
<div style="display:flex;flex-direction:column;gap:6px;font-size:13px"><span>✓ Page visits · languages · referrers</span><span>✓ Download click locations</span><span>✓ Metrics you defined (North Star)</span></div>
<div class="note">The first numbers fill in a few seconds. After that they refresh once a day, at 7 am.</div>
<div style="display:flex;gap:8px"><span class="btn p">View dashboard</span><span class="btn">Connect GitHub</span></div>
<div class="note">Remove a key any time in Settings → Manage connections.</div></div>
</div>
<div class="card" style="flex-direction:row;gap:32px;align-items:flex-start"><div style="flex:1"><div class="tag">What do the other two services take?</div></div>
<div style="flex:2;display:grid;grid-template-columns:1fr 1fr;gap:16px" class="note"><div><b style="color:var(--eq-color-text)">GitHub</b><br>A token with repository read access, from Settings → Developer settings → Tokens. Traffic shows only for an account with write access to the repository.</div><div><b style="color:var(--eq-color-text)">App Store Connect</b><br>An API key (.p8 file), key ID and issuer ID, from Users and Access → Integrations. Sales and Trends read access is enough.</div></div></div>
</main></div></body></html>`;

const dash=head('Dashboard')+`<div class="win">${side(true)}<main class="main">${bar}
<div class="grid g3">
<div class="card" style="grid-column:span 2"><h3>North Star · squads approving each week<span class="src">PostHog · refreshed today 07:00</span></h3>
<div style="display:flex;gap:32px;align-items:flex-end"><div><span class="big">14</span><span class="unit">squads</span><div>${d('up','3 (+27%)')}<span class="note"> last week 11</span></div></div><div style="flex:1">${sparkBars([2,3,3,5,4,6,7,6,8,11,11,14],72)}<div class="note" style="display:flex;justify-content:space-between"><span>Week of Aug 17</span><span>This week</span></div></div></div>
<div class="note">Squads that approved at least one result that week · <span style="color:var(--eq-color-accent);font-weight:600">Change definition</span></div></div>
${stat('Visits','PostHog · 07:00','1,284','page loads',d('up','8.2%'),line([120,140,170,160,210,190,230],[110,125,120,150,140,165,170]))}
</div>
<div class="grid g3">
<div class="card"><h3>Languages<span class="src">PostHog · 07:00</span></h3>${bars([['English',58,'742'],['Deutsch',22,'283'],['日本語',9,'112'],['Español',5,'64'],['Other',6,'83']])}<div class="note">By page loads · top of 8 languages</div></div>
<div class="card"><h3>Referrers<span class="src">PostHog · 07:00</span></h3>${bars([['Direct',40,'514'],['X',24,'308'],['GitHub',18,'231'],['LinkedIn',10,'128'],['Other',8,'103']])}<div class="note">By referrer · internal visits excluded</div></div>
<div class="card"><h3>Download click locations<span class="src">PostHog · 07:00</span></h3>${bars([['Hero button',62,'96'],['Top menu',22,'34'],['Install guide',12,'19'],['Footer',4,'7']])}<div class="note">macOS app (DMG) clicks in all <b style="color:var(--eq-color-text)">156</b> ${d('up','12%')}</div></div>
</div>
<div class="grid g3">
<div class="card" style="grid-column:span 2"><h3>GitHub traffic · stars<span class="src">GitHub · refreshed today 07:00</span></h3>
<div style="display:grid;grid-template-columns:repeat(3,1fr);gap:20px"><div><div class="note">Views</div><span class="big m">2,310</span><br>${d('up','5.1%')}</div><div><div class="note">Clones</div><span class="big m">418</span><br>${d('dn','2.3%')}</div><div><div class="note">Stars (total)</div><span class="big m">25</span><span class="unit">+4</span><br>${d('up','4')}</div></div>
${line([30,44,40,52,48,61,58],[28,35,41,38,45,47,50],600,56)}
<div class="note">GitHub only tells the last 14 days. The 30-day view carries on from the days equipe has kept.</div></div>
<div class="card"><h3>App downloads<span class="src warn">App Store Connect · as of Nov 1</span></h3><div><span class="big m">63</span><span class="unit">· iOS</span></div><div>${d('up','9 (+17%)')}<span class="note"> vs. previous period</span></div><div class="note">Apple reports a day late, so this shows up to yesterday. Android isn't out yet.</div></div>
</div></main></div></body></html>`;

const errcard=(t,src,body)=>`<div class="card" style="border-color:var(--eq-color-err)"><h3>${t}<span class="src bad">${src}</span></h3>${body}</div>`;
const state=head('Empty state·errors')+`<div class="win">${side(true)}<main class="main">${bar.replace('equipe-site','equipe-desktop')}
<div class="card" style="align-items:flex-start;gap:14px;padding:28px"><div class="tag">A · Empty state — no service connected yet</div>
<div style="font-weight:700;font-size:20px;line-height:28px;">Connect a service to see numbers</div>
<div class="note" style="max-width:560px;color:var(--eq-color-text)">Connect PostHog for visits and referrers, GitHub for stars and traffic, App Store Connect for app downloads — they show here.</div>
<div style="display:flex;gap:8px"><span class="btn p">Connect PostHog</span><span class="btn">GitHub</span><span class="btn">App Store Connect</span></div></div>
<div class="tag" style="margin-top:4px">B · Error — connected, but the numbers can't be fetched. The last values fetched stay, dimmed, with their time.</div>
<div class="banner err"><span style="font-size:18px">✕</span><div class="x"><b>Your PostHog key has expired</b><span class="note">Refused on Nov 2 at 07:00. Add a new key and it carries straight on.</span></div><span class="btn p">Add the key again</span></div>
<div class="banner wait"><span style="font-size:18px">⏳</span><div class="x"><b>GitHub's request limit was reached</b><span class="note">Trying again by itself at 14:20. These are the values from Nov 2, 07:00.</span></div><span class="btn">Try again now</span></div>
<div class="grid g3">
${errcard('Visits','PostHog · key expired',`<div style="opacity:.55"><span class="big m">1,284</span><span class="unit">page loads</span></div><div class="note">Last value · Nov 1, 07:00</div>`)}
${errcard('Download click locations','PostHog · key expired',`<div style="opacity:.55">${bars([['Hero button',62,'96'],['Top menu',22,'34'],['Install guide',12,'19']])}</div><div class="note">Last value · Nov 1, 07:00</div>`)}
<div class="card" style="border-color:var(--eq-color-wait)"><h3>GitHub stars<span class="src warn">GitHub · over the limit</span></h3><div style="opacity:.55"><span class="big m">25</span><span class="unit">stars</span></div><div class="note">Last value · Nov 1, 07:00 · trying again at 14:20</div></div>
</div></main></div></body></html>`;

const phone=(err)=>`<div style="width:390px;height:560px;border-radius:44px;border:10px solid var(--eq-color-text);background:var(--eq-color-bg);overflow:hidden;position:relative;padding:56px 16px 0;display:flex;flex-direction:column;gap:14px">
<div style="font-weight:700;font-size:28px;line-height:34px;">Metrics</div>
<div class="seg" style="align-self:flex-start"><b class="on" style="height:32px">equipe-site</b><b style="height:32px">equipe-app</b></div>
<div class="card" style="padding:16px;border-radius:16px;gap:10px"><h3 style="font-size:14px">This week · Oct 27–Nov 2</h3>
<div style="display:grid;grid-template-columns:1fr 1fr;gap:14px 12px">
<div><div class="note">Squads approving weekly</div><span class="big m">14</span><br>${d('up','3 · 27%')}</div>
<div><div class="note">Visits (page loads)</div><span class="big m">1,284</span><br>${d('up','8.2%')}</div>
<div><div class="note">Download clicks</div><span class="big m">156</span><br>${d('up','12%')}</div>
<div><div class="note">App downloads</div>${err?`<span class="big m" style="opacity:.55">63</span><br><span class="delta" style="color:var(--eq-color-err)">✕ Key expired</span>`:`<span class="big m">63</span><br>${d('dn','4 · 6%')}`}</div></div>
<div class="note" style="border-top:1px solid var(--eq-color-line);padding-top:8px">vs. last week · ${err?'App Store Connect values from Nov 1':'refreshed today 07:00'}</div></div>
${err?`<div class="banner err" style="padding:10px 12px"><div class="x"><b style="font-size:13px">Connect App Store Connect again</b><span class="note">Keys are added on the Mac. None is kept on the phone.</span></div></div>`:`<div class="note" style="padding:0 4px">equipe on your Mac gathers the numbers and sends them. No key is on the phone.</div>`}
</div>`;
const ph=head('Phone summary card')+`<div style="width:1280px;min-height:100vh;background:var(--eq-color-sunken);padding:48px 80px;display:grid;grid-template-columns:390px 390px 1fr;gap:48px;align-items:start">
<div><div class="tag" style="margin-bottom:10px">iOS · normal</div>${phone(false)}</div>
<div><div class="tag" style="margin-bottom:10px">iOS · some errors</div>${phone(true)}</div>
<div class="note" style="display:flex;flex-direction:column;gap:12px;padding-top:28px"><b style="color:var(--eq-color-text);font-size:15px">Rules</b>
<span>· At most 4 numbers on a card. With more, North Star takes the first slot.</span><span>· Every number has ▲▼ and a sign; colour only supports. A fall is not the error colour but <code>color.wait</code>.</span><span>· The period and refresh time go on one line under the card.</span><span>· The phone holds no keys. It only receives the numbers the Mac gathered.</span><span>· A slot in error dims its last value + ✕ and the reason.</span></div></div></body></html>`;
for(const [n,h] of Object.entries({connect,dash,state,ph}))fs.writeFileSync(n+'.html',h);
