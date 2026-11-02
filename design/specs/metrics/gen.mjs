import fs from 'fs';
const head=(t)=>`<!doctype html><html lang="ko"><head><meta charset="utf-8"><title>${t}</title><link rel="stylesheet" href="metrics.css"></head><body>`;
const side=(on)=>`<aside class="side"><div style="padding:4px 12px 12px;font-weight:700;font-size:15px">equipe<span style="color:var(--eq-color-dot)">.</span></div>
<h6>프로젝트</h6><div class="nav"><i></i>equipe-site</div><div class="nav"><i></i>equipe-app</div><div class="nav"><i></i>equipe-desktop</div>
<h6>보기</h6><div class="nav">대화</div><div class="nav">티켓</div><div class="nav ${on?'on':''}">지표</div><div class="nav">설정</div></aside>`;
const sparkBars=(vals,h=64,w=420,cur=true)=>{const m=Math.max(...vals);const bw=w/vals.length;return `<svg width="100%" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" style="height:${h}px">${vals.map((v,i)=>{const bh=Math.max(2,v/m*(h-4));return `<rect class="${i===vals.length-1&&cur?'fa':'fb'}" x="${i*bw+3}" y="${h-bh}" width="${bw-6}" height="${bh}" rx="3"/>`}).join('')}</svg>`};
const line=(a,b,w=300,h=56)=>{const m=Math.max(...a,...b);const pts=(v)=>v.map((x,i)=>`${(i/(v.length-1)*(w-4)+2).toFixed(1)},${(h-4-x/m*(h-10)).toFixed(1)}`).join(' ');return `<svg width="100%" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" style="height:${h}px"><polyline class="p" fill="none" stroke-width="2" stroke-dasharray="4 3" points="${pts(b)}"/><polyline class="a" fill="none" stroke-width="2.5" stroke-linejoin="round" points="${pts(a)}"/></svg>`};
const bars=(rows)=>rows.map(([n,p,v])=>`<div class="bar"><span>${n}</span><span class="t"><u style="width:${p}%"></u></span><span>${v}</span></div>`).join('');
const d=(k,t)=>`<span class="delta ${k}">${k==='up'?'▲':k==='dn'?'▼':'–'} ${t}</span>`;
const stat=(title,src,num,unit,delta,body='')=>`<div class="card"><h3>${title}<span class="src">${src}</span></h3><div><span class="big m">${num}</span><span class="unit">${unit}</span></div><div>${delta}<span class="note"> 지난 기간 대비</span></div>${body}</div>`;
const bar=`<div class="head"><div><h1>지표 · equipe-site</h1><div class="sub">10월 27일 ~ 11월 2일 · 지난 기간은 10월 20일 ~ 10월 26일</div></div><div style="display:flex;gap:12px;align-items:center"><div class="seg"><b class="on">7일</b><b>30일</b></div><span class="btn">새로고침</span><span class="btn">연결 관리</span></div></div>`;

const svc=(ic,n,desc,state)=>`<div style="display:flex;gap:12px;align-items:center;padding:12px 0;border-top:1px solid var(--eq-color-line)"><div class="ico">${ic}</div><div style="flex:1"><div style="font-weight:600;font-size:14px">${n}</div><div class="note">${desc}</div></div>${state}</div>`;
const connect=head('연결하기')+`<div class="win">${side(true)}<main class="main">
<div class="head"><div><h1>지표 연결하기</h1><div class="sub">서비스를 연결하면 숫자를 equipe 안에서 바로 봐요. 읽기만 하고, 아무것도 바꾸지 않아요.</div></div></div>
<div class="grid g3" style="align-items:start">
<div class="card"><div class="tag">1 · 서비스 고르기</div>
${svc('PH','PostHog','방문, 언어, 유입 경로, 클릭 위치','<span class="btn p">연결</span>')}
${svc('GH','GitHub','저장소 트래픽, 별','<span class="btn">연결</span>')}
${svc('AS','App Store Connect','앱 다운로드 수','<span class="btn">연결</span>')}
<div class="note">세 개 모두 연결하지 않아도 돼요. 연결한 것만 대시보드에 나와요.</div></div>
<div class="card"><div class="tag">2 · 키 넣기 · PostHog</div>
<div class="note" style="color:var(--eq-color-text)">PostHog → 설정 → 개인 API 키에서 <b>읽기 전용</b> 키를 만들어 붙여 넣어요.</div>
<div style="display:flex;flex-direction:column;gap:4px"><label class="l">개인 API 키</label><div class="field f">phx_••••••••••••••••<span style="margin-left:auto;width:1px;height:16px;background:var(--eq-color-text)"></span></div></div>
<div style="display:flex;flex-direction:column;gap:4px"><label class="l">프로젝트 ID</label><div class="field">700951</div><div class="note">PostHog 주소의 /project/ 뒤 숫자예요.</div></div>
<div style="display:flex;gap:8px"><span class="btn p">확인하고 연결</span><span class="btn g">뒤로</span></div>
<div class="note">🔒 키는 이 Mac에만 저장돼요. equipe 서버나 폰으로 보내지 않아요.</div></div>
<div class="card"><div class="tag">3 · 연결됨</div>
<div style="display:flex;gap:10px;align-items:center"><span class="badge ok">✓ 연결됨</span><b style="font-size:15px">PostHog</b></div>
<div class="note" style="color:var(--eq-color-text)">프로젝트 700951에서 읽을 수 있는 것</div>
<div style="display:flex;flex-direction:column;gap:6px;font-size:13px"><span>✓ 페이지 방문 · 언어 · 유입 경로</span><span>✓ 다운로드 클릭 위치</span><span>✓ 직접 정의한 지표 (North Star)</span></div>
<div class="note">첫 숫자는 몇 초 뒤에 채워져요. 이후 하루 한 번, 오전 7시에 갱신해요.</div>
<div style="display:flex;gap:8px"><span class="btn p">대시보드 보기</span><span class="btn">GitHub 연결하기</span></div>
<div class="note">키 삭제는 설정 → 연결 관리에서 언제든 해요.</div></div>
</div>
<div class="card" style="flex-direction:row;gap:32px;align-items:flex-start"><div style="flex:1"><div class="tag">다른 두 서비스는 무엇을 받아요?</div></div>
<div style="flex:2;display:grid;grid-template-columns:1fr 1fr;gap:16px" class="note"><div><b style="color:var(--eq-color-text)">GitHub</b><br>설정 → 개발자 설정 → 토큰에서 저장소 읽기 권한 토큰을 받아요. 트래픽은 저장소에 쓰기 권한이 있는 계정이어야 보여요.</div><div><b style="color:var(--eq-color-text)">App Store Connect</b><br>사용자 및 액세스 → 통합에서 API 키(.p8 파일), 키 ID, 발급자 ID를 받아요. 판매·트렌드 읽기 권한이면 돼요.</div></div></div>
</main></div></body></html>`;

const dash=head('대시보드')+`<div class="win">${side(true)}<main class="main">${bar}
<div class="grid g3">
<div class="card" style="grid-column:span 2"><h3>North Star · 주간 승인 스쿼드 수<span class="src">PostHog · 오늘 07:00 갱신</span></h3>
<div style="display:flex;gap:32px;align-items:flex-end"><div><span class="big">14</span><span class="unit">개 스쿼드</span><div>${d('up','3 (+27%)')}<span class="note"> 지난주 11개</span></div></div><div style="flex:1">${sparkBars([2,3,3,5,4,6,7,6,8,11,11,14],72)}<div class="note" style="display:flex;justify-content:space-between"><span>8월 17일 주</span><span>이번 주</span></div></div></div>
<div class="note">그 주에 결과물을 1건 이상 승인한 스쿼드 · <span style="color:var(--eq-color-accent);font-weight:600">정의 바꾸기</span></div></div>
${stat('방문','PostHog · 07:00','1,284','페이지 로드',d('up','8.2%'),line([120,140,170,160,210,190,230],[110,125,120,150,140,165,170]))}
</div>
<div class="grid g3">
<div class="card"><h3>언어<span class="src">PostHog · 07:00</span></h3>${bars([['English',58,'742'],['한국어',22,'283'],['日本語',9,'112'],['Español',5,'64'],['기타',6,'83']])}<div class="note">페이지 로드 수 기준 · 8개 언어 중 상위</div></div>
<div class="card"><h3>유입 경로<span class="src">PostHog · 07:00</span></h3>${bars([['직접',40,'514'],['X',24,'308'],['GitHub',18,'231'],['LinkedIn',10,'128'],['기타',8,'103']])}<div class="note">referrer 기준 · 내부 방문 제외</div></div>
<div class="card"><h3>다운로드 클릭 위치<span class="src">PostHog · 07:00</span></h3>${bars([['첫 화면 버튼',62,'96'],['상단 메뉴',22,'34'],['설치 안내',12,'19'],['맨 아래',4,'7']])}<div class="note">macOS 앱(DMG) 클릭 합계 <b style="color:var(--eq-color-text)">156</b> ${d('up','12%')}</div></div>
</div>
<div class="grid g3">
<div class="card" style="grid-column:span 2"><h3>GitHub 트래픽 · 별<span class="src">GitHub · 오늘 07:00 갱신</span></h3>
<div style="display:grid;grid-template-columns:repeat(3,1fr);gap:20px"><div><div class="note">조회</div><span class="big m">2,310</span><br>${d('up','5.1%')}</div><div><div class="note">클론</div><span class="big m">418</span><br>${d('dn','2.3%')}</div><div><div class="note">별 (누적)</div><span class="big m">25</span><span class="unit">+4</span><br>${d('up','4개')}</div></div>
${line([30,44,40,52,48,61,58],[28,35,41,38,45,47,50],600,56)}
<div class="note">GitHub는 최근 14일만 알려 줘요. 30일 보기에서는 equipe가 모아 둔 날부터 이어 보여 줘요.</div></div>
<div class="card"><h3>앱 다운로드<span class="src warn">App Store Connect · 11월 1일 기준</span></h3><div><span class="big m">63</span><span class="unit">건 · iOS</span></div><div>${d('up','9건 (+17%)')}<span class="note"> 지난 기간 대비</span></div><div class="note">Apple이 하루 늦게 알려 줘서 어제까지만 보여요. Android는 출시 전이에요.</div></div>
</div></main></div></body></html>`;

const errcard=(t,src,body)=>`<div class="card" style="border-color:var(--eq-color-err)"><h3>${t}<span class="src bad">${src}</span></h3>${body}</div>`;
const state=head('빈 상태·오류')+`<div class="win">${side(true)}<main class="main">${bar.replace('equipe-site','equipe-desktop')}
<div class="card" style="align-items:flex-start;gap:14px;padding:28px"><div class="tag">A · 빈 상태 — 아직 연결한 서비스가 없을 때</div>
<div style="font-weight:700;font-size:20px;line-height:28px;">숫자를 보려면 서비스를 하나 연결해요</div>
<div class="note" style="max-width:560px;color:var(--eq-color-text)">PostHog를 연결하면 방문과 유입 경로가, GitHub를 연결하면 별과 트래픽이, App Store Connect를 연결하면 앱 다운로드가 여기에 나와요.</div>
<div style="display:flex;gap:8px"><span class="btn p">PostHog 연결하기</span><span class="btn">GitHub</span><span class="btn">App Store Connect</span></div></div>
<div class="tag" style="margin-top:4px">B · 오류 — 연결은 했지만 숫자를 못 가져올 때. 마지막으로 가져온 값은 흐리게 두고 그 시각을 적어요.</div>
<div class="banner err"><span style="font-size:18px">✕</span><div class="x"><b>PostHog 키가 만료됐어요</b><span class="note">11월 2일 07:00에 거절됐어요. 새 키를 넣으면 바로 이어져요.</span></div><span class="btn p">키 다시 넣기</span></div>
<div class="banner wait"><span style="font-size:18px">⏳</span><div class="x"><b>GitHub 요청 한도에 닿았어요</b><span class="note">자동으로 14:20에 다시 시도해요. 지금은 11월 2일 07:00 값이에요.</span></div><span class="btn">지금 다시 시도</span></div>
<div class="grid g3">
${errcard('방문','PostHog · 키 만료',`<div style="opacity:.55"><span class="big m">1,284</span><span class="unit">페이지 로드</span></div><div class="note">마지막 값 · 11월 1일 07:00</div>`)}
${errcard('다운로드 클릭 위치','PostHog · 키 만료',`<div style="opacity:.55">${bars([['첫 화면 버튼',62,'96'],['상단 메뉴',22,'34'],['설치 안내',12,'19']])}</div><div class="note">마지막 값 · 11월 1일 07:00</div>`)}
<div class="card" style="border-color:var(--eq-color-wait)"><h3>GitHub 별<span class="src warn">GitHub · 한도 초과</span></h3><div style="opacity:.55"><span class="big m">25</span><span class="unit">개</span></div><div class="note">마지막 값 · 11월 1일 07:00 · 14:20에 다시 시도해요</div></div>
</div></main></div></body></html>`;

const phone=(err)=>`<div style="width:390px;height:560px;border-radius:44px;border:10px solid var(--eq-color-text);background:var(--eq-color-bg);overflow:hidden;position:relative;padding:56px 16px 0;display:flex;flex-direction:column;gap:14px">
<div style="font-weight:700;font-size:28px;line-height:34px;">지표</div>
<div class="seg" style="align-self:flex-start"><b class="on" style="height:32px">equipe-site</b><b style="height:32px">equipe-app</b></div>
<div class="card" style="padding:16px;border-radius:16px;gap:10px"><h3 style="font-size:14px">이번 주 · 10월 27일~11월 2일</h3>
<div style="display:grid;grid-template-columns:1fr 1fr;gap:14px 12px">
<div><div class="note">주간 승인 스쿼드</div><span class="big m">14</span><br>${d('up','3 · 27%')}</div>
<div><div class="note">방문 (페이지 로드)</div><span class="big m">1,284</span><br>${d('up','8.2%')}</div>
<div><div class="note">다운로드 클릭</div><span class="big m">156</span><br>${d('up','12%')}</div>
<div><div class="note">앱 다운로드</div>${err?`<span class="big m" style="opacity:.55">63</span><br><span class="delta" style="color:var(--eq-color-err)">✕ 키 만료</span>`:`<span class="big m">63</span><br>${d('dn','4건 · 6%')}`}</div></div>
<div class="note" style="border-top:1px solid var(--eq-color-line);padding-top:8px">지난주 대비 · ${err?'App Store Connect 11월 1일 값':'오늘 07:00 갱신'}</div></div>
${err?`<div class="banner err" style="padding:10px 12px"><div class="x"><b style="font-size:13px">App Store Connect를 다시 연결해요</b><span class="note">키는 Mac에서 넣어요. 폰에는 저장되지 않아요.</span></div></div>`:`<div class="note" style="padding:0 4px">숫자는 Mac의 equipe가 모아서 보내 줘요. 키는 폰에 없어요.</div>`}
</div>`;
const ph=head('폰 요약 카드')+`<div style="width:1280px;min-height:100vh;background:var(--eq-color-sunken);padding:48px 80px;display:grid;grid-template-columns:390px 390px 1fr;gap:48px;align-items:start">
<div><div class="tag" style="margin-bottom:10px">iOS · 평소</div>${phone(false)}</div>
<div><div class="tag" style="margin-bottom:10px">iOS · 일부 오류</div>${phone(true)}</div>
<div class="note" style="display:flex;flex-direction:column;gap:12px;padding-top:28px"><b style="color:var(--eq-color-text);font-size:15px">규칙</b>
<span>· 카드 한 장에 숫자 4개까지. 많으면 North Star가 첫 칸이에요.</span><span>· 숫자마다 ▲▼와 부호, 색은 보조예요. 하락은 오류색이 아니라 <code>color.wait</code>.</span><span>· 기준 기간과 갱신 시각은 카드 아래 한 줄.</span><span>· 폰에는 키가 없어요. Mac이 모은 숫자만 받아요.</span><span>· 오류인 칸은 마지막 값을 흐리게 + ✕ 이유.</span></div></div></body></html>`;
for(const [n,h] of Object.entries({connect,dash,state,ph}))fs.writeFileSync(n+'.html',h);
