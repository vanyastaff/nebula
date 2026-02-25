import { useState, useEffect, useRef } from "react";

const FONT_STYLE = `
  @import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@300;400;500;600&family=IBM+Plex+Sans:wght@300;400;500;600&display=swap');
  * { box-sizing: border-box; }
  ::-webkit-scrollbar { width: 4px; height: 4px; }
  ::-webkit-scrollbar-track { background: #0a0a0a; }
  ::-webkit-scrollbar-thumb { background: #222; border-radius: 2px; }
  @keyframes pulse-dot { 0%,100%{opacity:1;transform:scale(1)} 50%{opacity:.5;transform:scale(1.5)} }
  @keyframes stream-in { from{opacity:0;transform:translateY(-3px)} to{opacity:1;transform:translateY(0)} }
  @keyframes slide-up { from{opacity:0;transform:translateY(8px)} to{opacity:1;transform:translateY(0)} }
  .log-row { animation: stream-in .12s ease forwards; }
  .node-panel { animation: slide-up .15s ease forwards; }
`;

// ─── DATA ────────────────────────────────────────────────────────────────────

const EXECUTIONS = [
  { id:"ex_f3a",  wf:"Order Processing", trigger:"webhook", status:"completed", ms:300,  ago:"1s",  retries:0, nodes:"6/6", input:"0.8kb", output:"1.2kb" },
  { id:"ex_f51",  wf:"User Sync",        trigger:"cron",    status:"completed", ms:770,  ago:"25s", retries:0, nodes:"4/4", input:"0.4kb", output:"0.6kb" },
  { id:"ex_f68",  wf:"Email Campaign",   trigger:"webhook", status:"running",   ms:null, ago:"40s", retries:0, nodes:"2/5", input:"1.2kb", output:null },
  { id:"ex_f71",  wf:"Invoice Gen",      trigger:"manual",  status:"completed", ms:1710, ago:"6m",  retries:1, nodes:"7/7", input:"0.9kb", output:"3.2kb" },
  { id:"ex_f96",  wf:"Daily Report",     trigger:"webhook", status:"queued",    ms:null, ago:"8m",  retries:0, nodes:"0/8", input:"0.3kb", output:null },
  { id:"ex_fad",  wf:"Order Processing", trigger:"cron",    status:"failed",    ms:2650, ago:"9m",  retries:2, nodes:"3/6", input:"0.9kb", output:null,
    error:"nebula::resource::DatabaseError: connection timeout" },
  { id:"ex_fc4",  wf:"User Sync",        trigger:"webhook", status:"completed", ms:5120, ago:"10m", retries:0, nodes:"4/4", input:"0.4kb", output:"0.7kb" },
  { id:"ex_fdb",  wf:"Email Campaign",   trigger:"cron",    status:"completed", ms:5900, ago:"12m", retries:0, nodes:"5/5", input:"1.1kb", output:"1.4kb" },
  { id:"ex_ff2",  wf:"Invoice Gen",      trigger:"webhook", status:"completed", ms:4060, ago:"14m", retries:0, nodes:"7/7", input:"1.0kb", output:"2.9kb" },
  { id:"ex_1037", wf:"User Sync",        trigger:"cron",    status:"running",   ms:null, ago:"19m", retries:0, nodes:"1/4", input:"0.4kb", output:null },
  { id:"ex_104e", wf:"Email Campaign",   trigger:"webhook", status:"failed",    ms:1540, ago:"21m", retries:3, nodes:"2/5", input:"1.3kb", output:null,
    error:"nebula::resource::DatabaseError: connection timeout" },
  { id:"ex_1020", wf:"Order Processing", trigger:"webhook", status:"completed", ms:480,  ago:"23m", retries:0, nodes:"6/6", input:"0.8kb", output:"1.1kb" },
];

// ── Per-node rich data ────────────────────────────────────────────────────────
const NODE_DETAIL = {
  // ex_f3a
  "ex_f3a:1": {
    id:1, name:"Webhook Trigger", type:"TRIGGER", status:"ok",
    start:0, dur:12,
    input: null,
    output: { event:"order.created", headers:{ "content-type":"application/json","x-signature":"sha256=abc123" }, payload:{ orderId:"ord_1098", userId:"usr_4829", amount:149.99, currency:"USD" } },
    meta: { method:"POST", url:"/hooks/order", ip:"34.102.45.1", size:"284b" },
    logs: [
      { t:0, level:"INFO", msg:"Webhook received" },
      { t:1, level:"INFO", msg:"Signature validated" },
      { t:2, level:"INFO", msg:"Payload parsed — 284b" },
    ],
    retries: [],
  },
  "ex_f3a:2": {
    id:2, name:"Fetch User", type:"HTTP", status:"ok",
    start:12, dur:236,
    input: { userId:"usr_4829" },
    output: { id:"usr_4829", email:"alex@acme.com", plan:"premium", tier:"gold", createdAt:"2023-04-12T10:00:00Z" },
    meta: { method:"GET", url:"https://api.internal/users/usr_4829", statusCode:200, size:"312b", ttfb:"18ms" },
    logs: [
      { t:12,  level:"INFO", msg:"HTTP GET /users/usr_4829" },
      { t:248, level:"INFO", msg:"Response 200 OK — 312b in 236ms" },
    ],
    retries: [],
  },
  "ex_f3a:3": {
    id:3, name:"Check Premium", type:"CONDITION", status:"ok",
    start:248, dur:1,
    input: { user:{ plan:"premium", tier:"gold" } },
    output: { result:true, branch:"insert_order" },
    meta: { expression:"user.plan === 'premium'", evaluated:true, branch:"insert_order" },
    logs: [
      { t:248, level:"INFO", msg:"Evaluating: user.plan === 'premium'" },
      { t:249, level:"INFO", msg:"Result: true → branch insert_order" },
    ],
    retries: [],
  },
  "ex_f3a:4": {
    id:4, name:"Insert Order", type:"DB", status:"warn",
    start:249, dur:189,
    input: { userId:"usr_4829", amount:149.99, currency:"USD", items:[{ sku:"ITEM-001", qty:2 }] },
    output: { orderId:"ord_1098", status:"created", createdAt:"2024-02-22T14:23:01Z" },
    meta: { query:"INSERT INTO orders ...", table:"orders", rows:1, pool:"primary", poolLatency:"284ms", threshold:"200ms" },
    logs: [
      { t:249, level:"INFO",  msg:"Acquiring DB connection from pool" },
      { t:254, level:"WARN",  msg:"Pool latency elevated: 284ms RTT (threshold: 200ms)" },
      { t:438, level:"INFO",  msg:"INSERT executed — 1 row affected" },
      { t:438, level:"INFO",  msg:"orderId: ord_1098" },
    ],
    retries: [],
  },
  "ex_f3a:5": {
    id:5, name:"Send Email", type:"ACTION", status:"ok",
    start:438, dur:30,
    input: { to:"alex@acme.com", template:"order_confirmation", data:{ orderId:"ord_1098", amount:149.99 } },
    output: { messageId:"msg_8f3a2", queued:true, estimatedDelivery:"<1min" },
    meta: { provider:"SendGrid", template:"order_confirmation", queue:"transactional" },
    logs: [
      { t:438, level:"INFO", msg:"Email queued via SendGrid" },
      { t:468, level:"INFO", msg:"messageId: msg_8f3a2" },
    ],
    retries: [],
  },
  "ex_f3a:6": {
    id:6, name:"Notify Slack", type:"ACTION", status:"ok",
    start:468, dur:102,
    input: { channel:"#orders", text:"New order ord_1098 · $149.99 · usr_4829" },
    output: { ok:true, ts:"1708606981.123456", channel:"C04X8K2" },
    meta: { channel:"#orders", workspace:"acme-corp", method:"chat.postMessage" },
    logs: [
      { t:468, level:"INFO", msg:"POST slack chat.postMessage" },
      { t:570, level:"INFO", msg:"Response ok:true ts:1708606981.123456" },
    ],
    retries: [],
  },

  // ex_fad
  "ex_fad:1": {
    id:1, name:"Cron Trigger", type:"TRIGGER", status:"ok",
    start:0, dur:5,
    input: null,
    output: { schedule:"@every 5m", fireTime:"2024-02-22T14:14:22Z", jobId:"cron_order_sync" },
    meta: { schedule:"@every 5m", fireTime:"2024-02-22T14:14:22Z" },
    logs: [{ t:0, level:"INFO", msg:"Cron tick fired · schedule @every 5m" }],
    retries: [],
  },
  "ex_fad:2": {
    id:2, name:"Fetch Orders", type:"HTTP", status:"ok",
    start:5, dur:210,
    input: { status:"pending", since:"2024-02-22T14:09:22Z" },
    output: { orders:[{ id:"ord_2001" },{ id:"ord_2002" }], total:48, page:1 },
    meta: { method:"GET", url:"https://api.internal/orders?status=pending", statusCode:200, size:"8.4kb" },
    logs: [
      { t:5,   level:"INFO", msg:"GET /orders?status=pending" },
      { t:215, level:"INFO", msg:"Response 200 OK — 48 orders fetched" },
    ],
    retries: [],
  },
  "ex_fad:3": {
    id:3, name:"Validate Schema", type:"CONDITION", status:"ok",
    start:215, dur:2,
    input: { count:48, sample:{ id:"ord_2001", amount:99 } },
    output: { valid:true, passed:48, failed:0 },
    meta: { schema:"OrderSchema", validator:"zod@3.22" },
    logs: [{ t:215, level:"INFO", msg:"Schema validated: 48/48 records OK" }],
    retries: [],
  },
  "ex_fad:4": {
    id:4, name:"Insert Batch", type:"DB", status:"error",
    start:217, dur:2433,
    input: { orders:[{ id:"ord_2001" },{ id:"ord_2002" },{ "...":45 }], count:48 },
    output: null,
    meta: { query:"INSERT INTO orders_batch ...", table:"orders_batch", pool:"primary", attempt:3, maxAttempts:3 },
    logs: [
      { t:217,  level:"INFO",  msg:"Acquiring DB connection" },
      { t:217,  level:"WARN",  msg:"Pool latency elevated: 380ms RTT" },
      { t:890,  level:"WARN",  msg:"Retry 1/3 — connection reset by peer" },
      { t:1600, level:"WARN",  msg:"Retry 2/3 — connection reset by peer" },
      { t:2310, level:"ERROR", msg:"Retry 3/3 — connection timeout (5000ms)" },
      { t:2433, level:"ERROR", msg:"Activity failed: DatabaseError: connection timeout" },
    ],
    retries: [
      { attempt:1, at:"14:14:23.107", error:"connection reset by peer",    dur:"710ms" },
      { attempt:2, at:"14:14:23.817", error:"connection reset by peer",    dur:"710ms" },
      { attempt:3, at:"14:14:24.527", error:"connection timeout (5000ms)", dur:"710ms" },
    ],
    error: "nebula::resource::DatabaseError: connection timeout after 3 retries",
    stackTrace: `nebula::resource::DatabaseError: connection timeout
  at DbPool::acquire (nebula-db/src/pool.rs:142)
  at DbPool::retry_acquire (nebula-db/src/pool.rs:198)
  at Action::execute (nebula-core/src/action.rs:87)
  at WorkflowEngine::run_node (nebula-core/src/engine.rs:334)`,
  },
};

// Fallback node detail
const makeDefaultNode = (step, execId) => ({
  id: step.id, name: step.name, type: step.type, status: step.status,
  start: step.start, dur: step.dur,
  input: { param: "value" },
  output: step.status === "ok" ? { result: "success" } : null,
  meta: { type: step.type },
  logs: [
    { t: step.start, level: "INFO", msg: `${step.name} started` },
    { t: step.start + step.dur, level: step.status === "error" ? "ERROR" : "INFO", msg: `${step.name} ${step.status === "ok" ? "completed" : step.status}` },
  ],
  retries: [],
});

const STEPS = {
  "ex_f3a": [
    { id:1, name:"Webhook Trigger", type:"TRIGGER",   start:0,   dur:12,  status:"ok"    },
    { id:2, name:"Fetch User",      type:"HTTP",      start:12,  dur:236, status:"ok"    },
    { id:3, name:"Check Premium",   type:"CONDITION", start:248, dur:1,   status:"ok"    },
    { id:4, name:"Insert Order",    type:"DB",        start:249, dur:189, status:"warn"  },
    { id:5, name:"Send Email",      type:"ACTION",    start:438, dur:30,  status:"ok"    },
    { id:6, name:"Notify Slack",    type:"ACTION",    start:468, dur:102, status:"ok"    },
  ],
  "ex_fad": [
    { id:1, name:"Cron Trigger",    type:"TRIGGER",   start:0,   dur:5,    status:"ok"   },
    { id:2, name:"Fetch Orders",    type:"HTTP",      start:5,   dur:210,  status:"ok"   },
    { id:3, name:"Validate Schema", type:"CONDITION", start:215, dur:2,    status:"ok"   },
    { id:4, name:"Insert Batch",    type:"DB",        start:217, dur:2433, status:"error"},
  ],
  "default": [
    { id:1, name:"Trigger",  type:"TRIGGER", start:0,   dur:8,   status:"ok" },
    { id:2, name:"Process",  type:"ACTION",  start:8,   dur:320, status:"ok" },
    { id:3, name:"Persist",  type:"DB",      start:328, dur:190, status:"ok" },
    { id:4, name:"Notify",   type:"ACTION",  start:518, dur:80,  status:"ok" },
  ],
};

const LOGS = {
  "ex_f3a": [
    { t:0,   level:"INFO", msg:"Execution started" },
    { t:2,   level:"INFO", msg:"Webhook payload validated — 284b" },
    { t:12,  level:"INFO", msg:"Fetching user ID: usr_4829" },
    { t:248, level:"INFO", msg:"User found: plan=premium tier=gold" },
    { t:251, level:"INFO", msg:"Condition eval: user.tier==='premium' → true" },
    { t:254, level:"WARN", msg:"DB pool latency elevated: 284ms RTT" },
    { t:443, level:"INFO", msg:"Order inserted: ord_1098" },
    { t:446, level:"INFO", msg:"Slack notified: #orders" },
    { t:300, level:"INFO", msg:"Execution completed · 300ms · nodes 6/6" },
  ],
  "ex_fad": [
    { t:0,    level:"INFO",  msg:"Execution started" },
    { t:5,    level:"INFO",  msg:"Cron tick received · @every 5m" },
    { t:215,  level:"INFO",  msg:"Fetched 48 pending orders" },
    { t:217,  level:"WARN",  msg:"DB pool latency elevated: 380ms RTT" },
    { t:890,  level:"WARN",  msg:"Retry 1/3 — connection reset by peer" },
    { t:1600, level:"WARN",  msg:"Retry 2/3 — connection reset by peer" },
    { t:2310, level:"ERROR", msg:"Retry 3/3 — connection timeout" },
    { t:2650, level:"ERROR", msg:"Execution failed · 2.65s" },
  ],
  "default": [
    { t:0,   level:"INFO", msg:"Execution started" },
    { t:598, level:"INFO", msg:"Execution completed" },
  ],
};

const EVENT_HISTORY = {
  "ex_f3a": [
    { id:1,  type:"WorkflowExecutionStarted",   ts:"14:23:01.000", data:'{"workflowId":"ex_f3a"}' },
    { id:2,  type:"WorkflowTaskScheduled",      ts:"14:23:01.001", data:'{}' },
    { id:3,  type:"WorkflowTaskStarted",        ts:"14:23:01.002", data:'{"scheduledEventId":2}' },
    { id:4,  type:"WorkflowTaskCompleted",      ts:"14:23:01.012", data:'{"scheduledEventId":2}' },
    { id:5,  type:"ActivityTaskScheduled",      ts:"14:23:01.012", data:'{"activityType":"FetchUser","input":{"userId":"usr_4829"}}' },
    { id:6,  type:"ActivityTaskStarted",        ts:"14:23:01.013", data:'{"scheduledEventId":5}' },
    { id:7,  type:"ActivityTaskCompleted",      ts:"14:23:01.248", data:'{"result":{"plan":"premium","tier":"gold"}}' },
    { id:8,  type:"ActivityTaskScheduled",      ts:"14:23:01.249", data:'{"activityType":"InsertOrder"}' },
    { id:9,  type:"ActivityTaskStarted",        ts:"14:23:01.250", data:'{"scheduledEventId":8}' },
    { id:10, type:"ActivityTaskCompleted",      ts:"14:23:01.438", data:'{"result":{"orderId":"ord_1098"}}' },
    { id:11, type:"WorkflowExecutionCompleted", ts:"14:23:01.300", data:'{"output":{"status":"ok"}}' },
  ],
  "ex_fad": [
    { id:1,  type:"WorkflowExecutionStarted",  ts:"14:14:22.000", data:'{"workflowId":"ex_fad"}' },
    { id:2,  type:"WorkflowTaskScheduled",     ts:"14:14:22.001", data:'{}' },
    { id:3,  type:"ActivityTaskScheduled",     ts:"14:14:22.005", data:'{"activityType":"FetchOrders"}' },
    { id:4,  type:"ActivityTaskCompleted",     ts:"14:14:22.215", data:'{"result":{"count":48}}' },
    { id:5,  type:"ActivityTaskScheduled",     ts:"14:14:22.217", data:'{"activityType":"InsertBatch","retryPolicy":{"maxAttempts":3}}' },
    { id:6,  type:"ActivityTaskStarted",       ts:"14:14:22.217", data:'{"attempt":1}' },
    { id:7,  type:"ActivityTaskTimedOut",      ts:"14:14:23.107", data:'{"timeoutType":"ScheduleToClose"}' },
    { id:8,  type:"ActivityTaskStarted",       ts:"14:14:23.107", data:'{"attempt":2}' },
    { id:9,  type:"ActivityTaskTimedOut",      ts:"14:14:23.817", data:'{"timeoutType":"ScheduleToClose"}' },
    { id:10, type:"ActivityTaskStarted",       ts:"14:14:23.817", data:'{"attempt":3}' },
    { id:11, type:"ActivityTaskFailed",        ts:"14:14:24.527", data:'{"failure":{"message":"DatabaseError: connection timeout"}}' },
    { id:12, type:"WorkflowExecutionFailed",   ts:"14:14:24.650", data:'{"failure":{"message":"nebula::resource::DatabaseError"}}' },
  ],
};

const HEATMAP = (() => {
  const R=8, C=24;
  return Array.from({length:R}, (_,r) =>
    Array.from({length:C}, (_,c) => {
      const b = r<3?.7:r<5?.4:.1;
      return Math.max(0, Math.min(1, b + Math.sin(c*.5+r)*.2 + ((c===8||c===17)&&r>4?.5:0) + Math.random()*.08));
    })
  );
})();

const SPARKLINE = [88,91,89,94,92,96,94,97,95,98,96,98,97,99,98,97,99,98];

// ─── STYLE TOKENS ────────────────────────────────────────────────────────────
const S = {
  STATUS: {
    completed:{ c:"#22c55e", bg:"#0a1f10", border:"#14532d" },
    running:  { c:"#f59e0b", bg:"#1c0e00", border:"#451a03" },
    failed:   { c:"#ef4444", bg:"#1c0505", border:"#7f1d1d" },
    queued:   { c:"#6b7280", bg:"#111827", border:"#1f2937" },
    ok:       { c:"#22c55e", bg:"#0a1f10", border:"#14532d" },
    warn:     { c:"#f59e0b", bg:"#1c0e00", border:"#451a03" },
    error:    { c:"#ef4444", bg:"#1c0505", border:"#7f1d1d" },
  },
  TRIGGER: {
    webhook:{ bg:"#0f2238", border:"#1e3a5f", c:"#60a5fa" },
    cron:   { bg:"#0a1f10", border:"#1a3320", c:"#4ade80" },
    manual: { bg:"#1a0f35", border:"#2d1f4e", c:"#a78bfa" },
  },
  TYPE: {
    TRIGGER:  { bg:"#0f2238", border:"#1e3a5f", c:"#60a5fa" },
    HTTP:     { bg:"#0a1829", border:"#1e3a5f", c:"#38bdf8" },
    DB:       { bg:"#0a1420", border:"#164e63", c:"#7dd3fc" },
    CONDITION:{ bg:"#0a1510", border:"#14532d", c:"#86efac" },
    ACTION:   { bg:"#1a1205", border:"#78350f", c:"#fbbf24" },
  },
  EV: {
    WorkflowExecutionStarted: "#22c55e",
    WorkflowTaskScheduled:    "#374151",
    WorkflowTaskStarted:      "#374151",
    WorkflowTaskCompleted:    "#374151",
    ActivityTaskScheduled:    "#60a5fa",
    ActivityTaskStarted:      "#38bdf8",
    ActivityTaskCompleted:    "#22c55e",
    ActivityTaskTimedOut:     "#f59e0b",
    ActivityTaskFailed:       "#ef4444",
    WorkflowExecutionCompleted:"#22c55e",
    WorkflowExecutionFailed:  "#ef4444",
  },
};

const fmtMs = ms => ms >= 1000 ? `${(ms/1000).toFixed(1)}s` : `${ms}ms`;
const mono = { fontFamily:"IBM Plex Mono" };
const sans = { fontFamily:"IBM Plex Sans" };

// ─── ATOMS ───────────────────────────────────────────────────────────────────

function Dot({ status, pulse }) {
  const c = S.STATUS[status]?.c || "#6b7280";
  return (
    <span style={{ position:"relative", display:"inline-flex", width:8, height:8, flexShrink:0 }}>
      {pulse && <span style={{ position:"absolute", inset:0, borderRadius:9999, background:c,
        animation:"pulse-dot 1.5s ease-in-out infinite", opacity:.5 }} />}
      <span style={{ position:"absolute", inset:0, borderRadius:9999, background:c }} />
    </span>
  );
}

function Badge({ type, scheme }) {
  const s = (scheme==="trigger" ? S.TRIGGER : S.TYPE)[type] || { bg:"#111",border:"#222",c:"#555" };
  return (
    <span style={{ ...mono, background:s.bg, border:`1px solid ${s.border}`, color:s.c,
      fontSize:8, fontWeight:600, padding:"1px 5px", borderRadius:3,
      letterSpacing:1.5, textTransform:"uppercase", lineHeight:"14px", flexShrink:0 }}>
      {type}
    </span>
  );
}

function Sparkline({ data, color="#22c55e", W=80, H=22 }) {
  const max=Math.max(...data), min=Math.min(...data);
  const pts = data.map((v,i)=>`${(i/(data.length-1))*W},${H-((v-min)/(max-min+.001))*(H-3)-1}`).join(" ");
  return (
    <svg width={W} height={H}>
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
    </svg>
  );
}

function Divider() {
  return <div style={{ width:1, background:"#111", margin:"10px 0", flexShrink:0 }}/>;
}

function CopyBtn({ value }) {
  const [copied, setCopied] = useState(false);
  return (
    <button onClick={() => { navigator.clipboard?.writeText(JSON.stringify(value,null,2)); setCopied(true); setTimeout(()=>setCopied(false),1200); }}
      style={{ ...mono, fontSize:8, padding:"2px 6px", borderRadius:3, cursor:"pointer",
        background:"transparent", border:"1px solid #1f2937",
        color: copied ? "#22c55e" : "#6b7280", transition:"color .2s" }}>
      {copied ? "✓ copied" : "copy"}
    </button>
  );
}

// ─── JSON SYNTAX ─────────────────────────────────────────────────────────────

function JsonBlock({ data, maxH=200 }) {
  if (data === null || data === undefined) {
    return <span style={{ ...mono, fontSize:10, color:"#374151" }}>null</span>;
  }
  const text = JSON.stringify(data, null, 2);
  const colored = text.split("\n").map((ln, i) => {
    let color = "#9ca3af";
    if (/: "/.test(ln) || /^"/.test(ln.trim())) color = "#22c55e";
    else if (/: \d/.test(ln)) color = "#f59e0b";
    else if (/: null/.test(ln)) color = "#374151";
    else if (/: true|: false/.test(ln)) color = "#a78bfa";
    return <span key={i} style={{ display:"block", color }}>{ln}</span>;
  });
  return (
    <pre style={{ ...mono, fontSize:10, margin:0, lineHeight:1.65, maxHeight:maxH,
      overflow:"auto", padding:"10px 12px", background:"#060606",
      border:"1px solid #1a1a1a", borderRadius:5 }}>
      {colored}
    </pre>
  );
}

// ─── HEATMAP ─────────────────────────────────────────────────────────────────

function Heatmap() {
  const labels = ["0ms","50","100","200","400","800","1.6s","3.2s+"];
  const cell = v => v < .05 ? "transparent" : `rgba(${Math.round(v*220)},${Math.round((1-v)*160)},40,${.15+v*.85})`;
  return (
    <div>
      <div style={{ display:"flex", gap:4 }}>
        <div style={{ display:"flex", flexDirection:"column", gap:2, paddingTop:1 }}>
          {[...labels].reverse().map(l => (
            <div key={l} style={{ height:8, ...mono, fontSize:7, color:"#2d2d2d",
              textAlign:"right", paddingRight:4, lineHeight:"8px" }}>{l}</div>
          ))}
        </div>
        <div style={{ display:"flex", flexDirection:"column", gap:2, flex:1 }}>
          {[...HEATMAP].reverse().map((row,r) => (
            <div key={r} style={{ display:"flex", gap:2 }}>
              {row.map((v,c) => (
                <div key={c} style={{ flex:1, height:8, borderRadius:1,
                  background:cell(v), cursor:"crosshair", transition:"filter .1s" }}
                  onMouseEnter={e=>e.target.style.filter="brightness(1.6)"}
                  onMouseLeave={e=>e.target.style.filter=""}/>
              ))}
            </div>
          ))}
        </div>
      </div>
      <div style={{ display:"flex", justifyContent:"space-between", marginTop:3, paddingLeft:32 }}>
        {["-24h","-18h","-12h","-6h","now"].map(l => (
          <span key={l} style={{ ...mono, fontSize:7, color:"#1f1f1f" }}>{l}</span>
        ))}
      </div>
    </div>
  );
}

// ─── SLO ─────────────────────────────────────────────────────────────────────

function SloBar() {
  const cur=98.2, tgt=99.5, ok=cur>=tgt;
  return (
    <div>
      <div style={{ display:"flex", justifyContent:"space-between", alignItems:"baseline", marginBottom:5 }}>
        <span style={{ ...mono, fontSize:8, color:"#2d2d2d", letterSpacing:2 }}>SLO · 30d</span>
        <span style={{ ...mono, fontSize:12, color:ok?"#22c55e":"#ef4444", fontWeight:600 }}>{cur}%</span>
      </div>
      <div style={{ position:"relative", height:4, borderRadius:9999, background:"#111", overflow:"hidden" }}>
        <div style={{ height:"100%", borderRadius:9999, width:`${cur}%`,
          background:ok?"#22c55e":"#ef4444" }}/>
        <div style={{ position:"absolute", top:0, bottom:0, width:1, left:`${tgt}%`, background:"#f59e0b" }}/>
      </div>
      <div style={{ display:"flex", justifyContent:"space-between", marginTop:3 }}>
        <span style={{ ...mono, fontSize:7, color:"#2d2d2d" }}>target {tgt}%</span>
        <span style={{ ...mono, fontSize:7, color:ok?"#22c55e":"#ef4444" }}>
          {ok ? "✓ within budget" : `↓ ${(tgt-cur).toFixed(1)}% over`}
        </span>
      </div>
    </div>
  );
}

// ─── NODE DETAIL PANEL ───────────────────────────────────────────────────────

function NodeDetailPanel({ node, execId, onClose }) {
  const key = `${execId}:${node.id}`;
  const detail = NODE_DETAIL[key] || makeDefaultNode(node, execId);
  const [innerTab, setInnerTab] = useState("io");
  const st = S.STATUS[detail.status] || S.STATUS.ok;

  const ITABS = [
    { k:"io",    label:"Input / Output" },
    { k:"meta",  label:"Metadata" },
    { k:"logs",  label:`Logs (${detail.logs.length})` },
    ...(detail.retries.length > 0 ? [{ k:"retries", label:`Retries (${detail.retries.length})` }] : []),
    ...(detail.stackTrace ? [{ k:"trace", label:"Stack Trace" }] : []),
  ];

  return (
    <div className="node-panel" style={{
      position:"absolute", bottom:0, left:0, right:0,
      background:"#0c0c0c", borderTop:`1px solid ${st.border}`,
      zIndex:10, display:"flex", flexDirection:"column",
    }}>
      {/* Panel header */}
      <div style={{ display:"flex", alignItems:"center", gap:10, padding:"8px 16px",
        background:"#0a0a0a", borderBottom:"1px solid #111", flexShrink:0 }}>

        <div style={{ width:8, height:8, borderRadius:9999, background:st.c, flexShrink:0 }}/>

        <Badge type={detail.type} scheme="type"/>

        <span style={{ ...sans, fontSize:13, fontWeight:500, color:"#e5e7eb" }}>
          {detail.name}
        </span>

        <span style={{ ...mono, fontSize:10, color:"#374151" }}>
          node {detail.id}
        </span>

        <span style={{ ...mono, fontSize:9, color:st.c, background:st.bg,
          border:`1px solid ${st.border}`, padding:"1px 6px", borderRadius:3 }}>
          {detail.status}
        </span>

        <div style={{ flex:1 }}/>

        {/* Timing */}
        <div style={{ display:"flex", gap:16 }}>
          {[
            ["start",  `${detail.start}ms`],
            ["dur",    fmtMs(detail.dur)],
            ["end",    fmtMs(detail.start + detail.dur)],
          ].map(([k,v]) => (
            <div key={k} style={{ display:"flex", flexDirection:"column", alignItems:"flex-end", gap:1 }}>
              <span style={{ ...mono, fontSize:11, fontWeight:600, color:"#e5e7eb" }}>{v}</span>
              <span style={{ ...mono, fontSize:7, color:"#2d2d2d" }}>{k}</span>
            </div>
          ))}
        </div>

        <div style={{ width:1, height:16, background:"#1a1a1a" }}/>

        <button onClick={onClose} style={{ ...mono, fontSize:11, color:"#4b5563",
          background:"transparent", border:"none", cursor:"pointer", padding:"2px 6px",
          lineHeight:1 }}>✕</button>
      </div>

      {/* Inner tabs */}
      <div style={{ display:"flex", padding:"0 16px", borderBottom:"1px solid #111", flexShrink:0 }}>
        {ITABS.map(t => (
          <button key={t.k} onClick={()=>setInnerTab(t.k)} style={{
            ...mono, fontSize:9, padding:"6px 12px", letterSpacing:.5,
            background:"transparent", border:"none", cursor:"pointer",
            borderBottom:`2px solid ${innerTab===t.k?"#f59e0b":"transparent"}`,
            color: innerTab===t.k?"#fbbf24":"#374151",
            textTransform:"uppercase", transition:"all .12s",
          }}>{t.label}</button>
        ))}
      </div>

      {/* Tab content */}
      <div style={{ display:"flex", gap:0, overflow:"hidden", height:220 }}>

        {/* ── I/O ── */}
        {innerTab==="io" && (
          <div style={{ display:"flex", flex:1, overflow:"hidden" }}>

            {/* Input */}
            <div style={{ flex:1, display:"flex", flexDirection:"column",
              borderRight:"1px solid #111", overflow:"hidden" }}>
              <div style={{ display:"flex", alignItems:"center", gap:8, padding:"8px 12px",
                borderBottom:"1px solid #0d0d0d", flexShrink:0 }}>
                <span style={{ ...mono, fontSize:8, color:"#374151", letterSpacing:2 }}>INPUT</span>
                <span style={{ ...mono, fontSize:8, color:"#1f2937" }}>
                  {detail.input ? `${JSON.stringify(detail.input).length}b` : "—"}
                </span>
                <div style={{ flex:1 }}/>
                {detail.input && <CopyBtn value={detail.input}/>}
              </div>
              <div style={{ flex:1, overflow:"auto", padding:12 }}>
                {detail.input
                  ? <JsonBlock data={detail.input} maxH={160}/>
                  : <div style={{ ...mono, fontSize:10, color:"#2d2d2d", padding:"8px 0" }}>
                      No input — this is a trigger node
                    </div>
                }
              </div>
            </div>

            {/* Output */}
            <div style={{ flex:1, display:"flex", flexDirection:"column", overflow:"hidden" }}>
              <div style={{ display:"flex", alignItems:"center", gap:8, padding:"8px 12px",
                borderBottom:"1px solid #0d0d0d", flexShrink:0 }}>
                <span style={{ ...mono, fontSize:8, color:"#374151", letterSpacing:2 }}>OUTPUT</span>
                <span style={{ ...mono, fontSize:8, color:"#1f2937" }}>
                  {detail.output ? `${JSON.stringify(detail.output).length}b` : "—"}
                </span>
                {detail.status === "error" && (
                  <span style={{ ...mono, fontSize:8, color:"#ef4444",
                    background:"#450a0a", border:"1px solid #7f1d1d",
                    padding:"1px 5px", borderRadius:3 }}>failed</span>
                )}
                <div style={{ flex:1 }}/>
                {detail.output && <CopyBtn value={detail.output}/>}
              </div>
              <div style={{ flex:1, overflow:"auto", padding:12 }}>
                {detail.output
                  ? <JsonBlock data={detail.output} maxH={160}/>
                  : detail.error
                    ? <div style={{ padding:"8px 12px", background:"#180808",
                        border:"1px solid #7f1d1d33", borderRadius:5 }}>
                        <div style={{ ...mono, fontSize:9, color:"#ef4444", marginBottom:4, letterSpacing:.5 }}>
                          ERROR
                        </div>
                        <div style={{ ...mono, fontSize:10, color:"#fca5a5", lineHeight:1.5 }}>
                          {detail.error}
                        </div>
                      </div>
                    : <div style={{ ...mono, fontSize:10, color:"#2d2d2d", padding:"8px 0" }}>
                        No output produced
                      </div>
                }
              </div>
            </div>
          </div>
        )}

        {/* ── METADATA ── */}
        {innerTab==="meta" && (
          <div style={{ flex:1, overflow:"auto", padding:16 }}>
            <div style={{ display:"grid", gridTemplateColumns:"1fr 1fr", gap:12 }}>
              {/* Top KV */}
              <div style={{ background:"#0a0a0a", border:"1px solid #1a1a1a",
                borderRadius:6, padding:12 }}>
                <div style={{ ...mono, fontSize:8, color:"#2d2d2d", letterSpacing:2, marginBottom:10 }}>
                  NODE INFO
                </div>
                {[
                  ["Node ID",   `#${detail.id}`],
                  ["Type",      detail.type],
                  ["Status",    detail.status],
                  ["Start",     `${detail.start}ms`],
                  ["Duration",  fmtMs(detail.dur)],
                  ["End",       fmtMs(detail.start+detail.dur)],
                ].map(([k,v]) => (
                  <div key={k} style={{ display:"flex", justifyContent:"space-between",
                    padding:"3px 0", borderBottom:"1px solid #111" }}>
                    <span style={{ ...mono, fontSize:9, color:"#374151" }}>{k}</span>
                    <span style={{ ...mono, fontSize:10, color:"#9ca3af" }}>{v}</span>
                  </div>
                ))}
              </div>

              {/* Type-specific meta */}
              <div style={{ background:"#0a0a0a", border:"1px solid #1a1a1a",
                borderRadius:6, padding:12 }}>
                <div style={{ ...mono, fontSize:8, color:"#2d2d2d", letterSpacing:2, marginBottom:10 }}>
                  {detail.type} PARAMS
                </div>
                {Object.entries(detail.meta).map(([k,v]) => (
                  <div key={k} style={{ display:"flex", justifyContent:"space-between",
                    padding:"3px 0", borderBottom:"1px solid #111", gap:8 }}>
                    <span style={{ ...mono, fontSize:9, color:"#374151", flexShrink:0 }}>{k}</span>
                    <span style={{ ...mono, fontSize:9, color: k.toLowerCase().includes("latency")&&v>"200ms" ? "#f59e0b" : "#6b7280",
                      textAlign:"right", overflow:"hidden", textOverflow:"ellipsis", whiteSpace:"nowrap" }}>
                      {String(v)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        {/* ── LOGS ── */}
        {innerTab==="logs" && (
          <div style={{ flex:1, overflow:"auto" }}>
            {detail.logs.map((log,i) => (
              <div key={i} style={{
                display:"flex", gap:8, padding:"5px 16px", borderBottom:"1px solid #0a0a0a",
                background: log.level==="ERROR"?"rgba(100,20,20,.15)":log.level==="WARN"?"rgba(70,40,5,.15)":"transparent",
              }}>
                <span style={{ ...mono, fontSize:9, color:"#2d2d2d", width:40, textAlign:"right", flexShrink:0 }}>
                  {log.t}ms
                </span>
                <span style={{ ...mono, fontSize:9, width:42, fontWeight:600, flexShrink:0,
                  color:log.level==="ERROR"?"#ef4444":log.level==="WARN"?"#f59e0b":"#2d2d2d" }}>
                  {log.level}
                </span>
                <span style={{ ...mono, fontSize:10, lineHeight:1.5,
                  color:log.level==="ERROR"?"#fca5a5":log.level==="WARN"?"#fde68a":"#6b7280" }}>
                  {log.msg}
                </span>
              </div>
            ))}
          </div>
        )}

        {/* ── RETRIES ── */}
        {innerTab==="retries" && (
          <div style={{ flex:1, overflow:"auto", padding:16 }}>
            <div style={{ marginBottom:12, ...mono, fontSize:9, color:"#374151" }}>
              {detail.retries.length} attempts · policy: maxAttempts={detail.meta?.attempt || detail.retries.length}
            </div>
            <div style={{ display:"flex", flexDirection:"column", gap:6 }}>
              {detail.retries.map(r => (
                <div key={r.attempt} style={{ background:"#0a0a0a",
                  border:"1px solid #7f1d1d33", borderRadius:5, padding:"8px 12px" }}>
                  <div style={{ display:"flex", alignItems:"center", gap:8, marginBottom:4 }}>
                    <span style={{ ...mono, fontSize:9, fontWeight:600, color:"#f97316" }}>
                      Attempt {r.attempt}
                    </span>
                    <span style={{ ...mono, fontSize:8, color:"#374151" }}>{r.at}</span>
                    <div style={{ flex:1 }}/>
                    <span style={{ ...mono, fontSize:8, color:"#4b5563" }}>{r.dur}</span>
                  </div>
                  <div style={{ ...mono, fontSize:10, color:"#fca5a5" }}>{r.error}</div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* ── STACK TRACE ── */}
        {innerTab==="trace" && (
          <div style={{ flex:1, overflow:"auto", padding:16 }}>
            <pre style={{ ...mono, fontSize:10, color:"#6b7280", lineHeight:1.7,
              background:"#060606", border:"1px solid #1a1a1a",
              borderRadius:5, padding:12, margin:0, overflow:"auto" }}>
              {(detail.stackTrace || "").split("\n").map((ln,i) => (
                <span key={i} style={{ display:"block",
                  color: i===0?"#ef4444":ln.trim().startsWith("at nebula")?"#9ca3af":"#4b5563" }}>
                  {ln}
                </span>
              ))}
            </pre>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── GANTT + NODE SELECTION ───────────────────────────────────────────────────

function Gantt({ steps, total, execId, selectedNode, onSelectNode }) {
  const T = total || steps.reduce((a,s)=>Math.max(a,s.start+s.dur),0);

  return (
    <div style={{ padding:"16px 16px 0", overflow:"hidden" }}>
      {/* Axis */}
      <div style={{ display:"flex", marginBottom:10, paddingLeft:164 }}>
        {[0,25,50,75,100].map(p => (
          <div key={p} style={{ flex:1, position:"relative" }}>
            <div style={{ position:"absolute", left:0, top:0, bottom:0, width:1, background:"#141414" }}/>
            <span style={{ ...mono, fontSize:8, color:"#1f1f1f", paddingLeft:2 }}>
              {Math.round(T*p/100)}ms
            </span>
          </div>
        ))}
      </div>

      {/* Steps */}
      <div style={{ display:"flex", flexDirection:"column", gap:5 }}>
        {steps.map(s => {
          const lp = (s.start/T)*100;
          const wp = Math.max((s.dur/T)*100, .4);
          const isSel = selectedNode?.id === s.id;
          const barBg = s.status==="error"?"#200808":s.status==="warn"?"#1a0e00":"#080f08";
          const barBd = s.status==="error"?"#ef4444":s.status==="warn"?"#f59e0b":"#22c55e";

          return (
            <div key={s.id} style={{ display:"flex", alignItems:"center", gap:8 }}
              onClick={() => onSelectNode(isSel ? null : s)}>

              {/* Label */}
              <div style={{ width:160, flexShrink:0, display:"flex",
                alignItems:"center", gap:6, justifyContent:"flex-end", cursor:"pointer" }}>
                <span style={{ ...sans, fontSize:11, color: isSel?"#e5e7eb":"#4b5563",
                  transition:"color .1s", overflow:"hidden", textOverflow:"ellipsis", whiteSpace:"nowrap" }}>
                  {s.name}
                </span>
                <Badge type={s.type} scheme="type"/>
              </div>

              {/* Track */}
              <div style={{ flex:1, height:26, borderRadius:4, background:"#080808",
                position:"relative", overflow:"hidden", cursor:"pointer",
                border:`1px solid ${isSel ? barBd+"66" : "transparent"}`,
                transition:"border-color .12s",
              }}
                onMouseEnter={e=>{ if(!isSel) e.currentTarget.style.background="#0d0d0d"; }}
                onMouseLeave={e=>{ if(!isSel) e.currentTarget.style.background="#080808"; }}>

                {[25,50,75].map(p=>(
                  <div key={p} style={{ position:"absolute", top:0, bottom:0, width:1,
                    left:`${p}%`, background:"#0f0f0f" }}/>
                ))}

                {/* Bar */}
                <div style={{
                  position:"absolute", top:2, bottom:2, borderRadius:3,
                  left:`${lp}%`, width:`${wp}%`,
                  background:barBg, border:`1px solid ${barBd}`,
                  boxShadow: isSel ? `0 0 10px ${barBd}40` : "none",
                  transition:"box-shadow .15s",
                }}>
                  {wp > 6 && (
                    <span style={{ ...mono, fontSize:9, color:"#9ca3af",
                      position:"absolute", left:6, top:"50%", transform:"translateY(-50%)" }}>
                      {fmtMs(s.dur)}
                    </span>
                  )}
                </div>

                {/* Click hint */}
                {isSel && (
                  <div style={{ position:"absolute", right:6, top:"50%", transform:"translateY(-50%)" }}>
                    <span style={{ ...mono, fontSize:8, color:barBd }}>▼ details</span>
                  </div>
                )}
              </div>

              <span style={{ ...mono, fontSize:10, width:44, textAlign:"right", flexShrink:0,
                color:s.status==="error"?"#ef4444":s.status==="warn"?"#f59e0b":"#2d2d2d" }}>
                {fmtMs(s.dur)}
              </span>
            </div>
          );
        })}
      </div>

      {/* Heatmap */}
      <div style={{ marginTop:20, paddingTop:16, borderTop:"1px solid #111" }}>
        <div style={{ display:"flex", alignItems:"center", gap:8, marginBottom:8 }}>
          <span style={{ ...mono, fontSize:8, color:"#2d2d2d", letterSpacing:2 }}>
            LATENCY DISTRIBUTION · 24h
          </span>
          <span style={{ ...mono, fontSize:8, color:"#22c55e" }}>▲ this run</span>
        </div>
        <Heatmap/>
      </div>
    </div>
  );
}

// ─── LOGS ─────────────────────────────────────────────────────────────────────

function Logs({ logs }) {
  const [lv, setLv] = useState("ALL");
  const [q, setQ]   = useState("");
  const rows = logs.filter(l=>(lv==="ALL"||l.level===lv)&&l.msg.toLowerCase().includes(q.toLowerCase()));
  const ct = { WARN:logs.filter(l=>l.level==="WARN").length, ERROR:logs.filter(l=>l.level==="ERROR").length };

  return (
    <div style={{ display:"flex", flexDirection:"column", height:"100%" }}>
      <div style={{ display:"flex", alignItems:"center", gap:6, padding:"8px 12px",
        borderBottom:"1px solid #111", flexShrink:0 }}>
        <div style={{ display:"flex", alignItems:"center", gap:6, flex:1, padding:"4px 8px",
          background:"#0a0a0a", border:"1px solid #1a1a1a", borderRadius:4 }}>
          <span style={{ color:"#2d2d2d" }}>⌕</span>
          <input value={q} onChange={e=>setQ(e.target.value)} placeholder="filter logs..."
            style={{ ...mono, fontSize:11, color:"#9ca3af", flex:1,
              background:"transparent", border:"none", outline:"none" }}/>
        </div>
        {["ALL","INFO","WARN","ERROR"].map(f=>(
          <button key={f} onClick={()=>setLv(f)} style={{
            ...mono, fontSize:9, padding:"2px 7px", borderRadius:3, cursor:"pointer",
            background: lv===f?"#1f2937":"transparent",
            border:`1px solid ${lv===f?"#374151":"transparent"}`,
            color: lv===f?"#e5e7eb":f==="WARN"?"#d97706":f==="ERROR"?"#ef4444":"#4b5563",
          }}>
            {f}{ct[f]?" "+ct[f]:""}
          </button>
        ))}
      </div>
      <div style={{ flex:1, overflowY:"auto" }}>
        {rows.map((log,i)=>(
          <div key={i} className="log-row" style={{
            display:"flex", gap:8, padding:"5px 12px", borderBottom:"1px solid #0a0a0a",
            background: log.level==="ERROR"?"rgba(100,20,20,.15)":log.level==="WARN"?"rgba(70,40,5,.15)":"transparent",
            animationDelay:`${i*8}ms`,
          }}>
            <span style={{ ...mono, fontSize:9, color:"#2d2d2d", width:40, textAlign:"right", flexShrink:0 }}>{log.t}ms</span>
            <span style={{ ...mono, fontSize:9, width:42, fontWeight:600, flexShrink:0,
              color:log.level==="ERROR"?"#ef4444":log.level==="WARN"?"#f59e0b":"#2d2d2d" }}>{log.level}</span>
            <span style={{ ...mono, fontSize:10, lineHeight:1.5,
              color:log.level==="ERROR"?"#fca5a5":log.level==="WARN"?"#fde68a":"#6b7280" }}>{log.msg}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── EVENT HISTORY ────────────────────────────────────────────────────────────

function EventHist({ events }) {
  const [exp, setExp] = useState(null);
  const [sort, setSort] = useState("asc");
  const rows = sort==="asc" ? events : [...events].reverse();
  if (!events.length) return (
    <div style={{ display:"flex", alignItems:"center", justifyContent:"center",
      height:"100%", ...mono, fontSize:11, color:"#2d2d2d" }}>No event history</div>
  );
  return (
    <div style={{ display:"flex", flexDirection:"column", height:"100%" }}>
      <div style={{ display:"flex", alignItems:"center", gap:8, padding:"8px 12px",
        borderBottom:"1px solid #111", flexShrink:0 }}>
        <span style={{ ...mono, fontSize:9, color:"#2d2d2d" }}>{events.length} events</span>
        <div style={{ flex:1 }}/>
        <button onClick={()=>setSort(s=>s==="asc"?"desc":"asc")} style={{
          ...mono, fontSize:9, padding:"2px 6px", borderRadius:3, cursor:"pointer",
          background:"#1f2937", border:"1px solid #374151", color:"#6b7280",
        }}>{sort==="asc"?"↑ asc":"↓ desc"}</button>
      </div>
      <div style={{ flex:1, overflowY:"auto" }}>
        {rows.map(ev=>{
          const c = S.EV[ev.type] || "#374151";
          const isExp = exp===ev.id;
          return (
            <div key={ev.id} onClick={()=>setExp(isExp?null:ev.id)}
              style={{ borderBottom:"1px solid #0a0a0a", cursor:"pointer" }}
              onMouseEnter={e=>e.currentTarget.style.background="#ffffff06"}
              onMouseLeave={e=>e.currentTarget.style.background=""}>
              <div style={{ display:"flex", alignItems:"center", gap:8, padding:"6px 12px" }}>
                <span style={{ ...mono, fontSize:9, color:"#1f1f1f", width:18, textAlign:"right" }}>{ev.id}</span>
                <div style={{ width:6, height:6, borderRadius:9999, background:c, flexShrink:0 }}/>
                <span style={{ ...mono, fontSize:10, color:c, flex:1, letterSpacing:.3 }}>
                  {ev.type.replace(/([A-Z])/g," $1").trim()}
                </span>
                <span style={{ ...mono, fontSize:9, color:"#1f1f1f" }}>{ev.ts}</span>
                <span style={{ ...mono, fontSize:9, color:"#374151" }}>{isExp?"▲":"▼"}</span>
              </div>
              {isExp && (
                <div style={{ padding:"0 12px 8px", paddingLeft:44 }}>
                  <JsonBlock data={JSON.parse(ev.data)} maxH={100}/>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ─── RIGHT PANEL ─────────────────────────────────────────────────────────────

function RightPanel({ exec }) {
  const workers = [
    { id:"wrk-1", status:"active", q:3, cpu:"18%" },
    { id:"wrk-2", status:"active", q:1, cpu:"7%"  },
    { id:"wrk-3", status:"active", q:0, cpu:"3%"  },
    { id:"wrk-4", status:"idle",   q:0, cpu:"0%"  },
  ];
  const related = EXECUTIONS.filter(e=>e.wf===exec.wf&&e.id!==exec.id).slice(0,5);
  const M = ({k,v,c="#6b7280"}) => (
    <div style={{ display:"flex", justifyContent:"space-between", padding:"3px 0", borderBottom:"1px solid #0d0d0d" }}>
      <span style={{ ...mono, fontSize:9, color:"#2d2d2d" }}>{k}</span>
      <span style={{ ...mono, fontSize:10, color:c }}>{v}</span>
    </div>
  );

  return (
    <div style={{ display:"flex", flexDirection:"column", height:"100%", overflowY:"auto" }}>

      <div style={{ padding:12, borderBottom:"1px solid #111" }}>
        <div style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:8 }}>METADATA</div>
        <M k="ID"      v={exec.id}      c="#60a5fa"/>
        <M k="Workflow" v={exec.wf}     c="#e5e7eb"/>
        <M k="Trigger" v={exec.trigger} c="#a78bfa"/>
        <M k="Nodes"   v={exec.nodes}   c="#22c55e"/>
        <M k="Input"   v={exec.input}/>
        <M k="Output"  v={exec.output||"—"}/>
        <M k="Retries" v={exec.retries} c={exec.retries>0?"#f59e0b":"#6b7280"}/>
        <M k="Tenant"  v="default"/>
      </div>

      <div style={{ padding:12, borderBottom:"1px solid #111" }}><SloBar/></div>

      <div style={{ padding:12, borderBottom:"1px solid #111" }}>
        <div style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:8 }}>PERCENTILES</div>
        {[["P50","247ms","#22c55e","25%"],["P90","612ms","#f59e0b","60%"],["P95","891ms","#f97316","85%"],["P99","2.1s","#ef4444","100%"]].map(([p,v,c,w])=>(
          <div key={p} style={{ display:"flex", alignItems:"center", gap:6, padding:"2px 0" }}>
            <span style={{ ...mono, fontSize:9, color:"#374151", width:22 }}>{p}</span>
            <div style={{ flex:1, height:3, borderRadius:9999, background:"#111", overflow:"hidden" }}>
              <div style={{ height:"100%", borderRadius:9999, width:w, background:c }}/>
            </div>
            <span style={{ ...mono, fontSize:9, color:c, width:30, textAlign:"right" }}>{v}</span>
          </div>
        ))}
      </div>

      <div style={{ padding:12, borderBottom:"1px solid #111" }}>
        <div style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:8 }}>WORKERS</div>
        {workers.map(w=>(
          <div key={w.id} style={{ display:"flex", alignItems:"center", gap:6, padding:"2px 0" }}>
            <div style={{ width:5,height:5,borderRadius:9999,flexShrink:0,
              background:w.status==="active"?"#22c55e":"#1f2937" }}/>
            <span style={{ ...mono, fontSize:9, color:"#374151", flex:1 }}>{w.id}</span>
            <span style={{ ...mono, fontSize:8, color:"#1f2937" }}>q{w.q}</span>
            <span style={{ ...mono, fontSize:9, color:"#2d2d2d" }}>{w.cpu}</span>
          </div>
        ))}
      </div>

      <div style={{ padding:12, borderBottom:"1px solid #111" }}>
        <div style={{ display:"flex", justifyContent:"space-between", alignItems:"center", marginBottom:8 }}>
          <span style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3 }}>SUCCESS · 20m</span>
          <span style={{ ...mono, fontSize:12, color:"#22c55e", fontWeight:600 }}>98%</span>
        </div>
        <Sparkline data={SPARKLINE} color="#22c55e" W={176} H={26}/>
      </div>

      <div style={{ padding:12 }}>
        <div style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:8 }}>
          RELATED · {exec.wf.toUpperCase()}
        </div>
        {related.map(r=>(
          <div key={r.id} style={{ display:"flex", alignItems:"center", gap:6, padding:"3px 0" }}>
            <Dot status={r.status} pulse={r.status==="running"}/>
            <span style={{ ...mono, fontSize:9, color:"#374151", flex:1 }}>{r.id}</span>
            <span style={{ ...mono, fontSize:8, color:"#2d2d2d" }}>{r.ago}</span>
            {r.ms && <span style={{ ...mono, fontSize:9, color:"#1f2937" }}>{fmtMs(r.ms)}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}

// ─── ROOT ─────────────────────────────────────────────────────────────────────

export default function MonitorPage() {
  const [sel, setSel]           = useState(EXECUTIONS[0]);
  const [filter, setFilter]     = useState("all");
  const [q, setQ]               = useState("");
  const [tab, setTab]           = useState("timeline");
  const [isLive, setIsLive]     = useState(true);
  const [selectedNode, setSelectedNode] = useState(null);

  const ct = {
    all:       EXECUTIONS.length,
    running:   EXECUTIONS.filter(e=>e.status==="running").length,
    completed: EXECUTIONS.filter(e=>e.status==="completed").length,
    failed:    EXECUTIONS.filter(e=>e.status==="failed").length,
    queued:    EXECUTIONS.filter(e=>e.status==="queued").length,
  };

  const list = EXECUTIONS.filter(e =>
    (filter==="all"||e.status===filter) &&
    (!q || e.id.includes(q)||e.wf.toLowerCase().includes(q.toLowerCase()))
  );

  const steps  = STEPS[sel.id]  || STEPS.default;
  const logs   = LOGS[sel.id]   || LOGS.default;
  const events = EVENT_HISTORY[sel.id] || [];
  const total  = sel.ms || steps.reduce((a,s)=>Math.max(a,s.start+s.dur),0);

  const TABS = ["timeline","logs","events"];

  const navBtn = (special) => ({
    ...mono, fontSize:9, padding:"3px 9px", borderRadius:4, cursor:"pointer",
    background: special==="green" ? "#0a1f10" : "transparent",
    border:`1px solid ${special==="green"?"#14532d":special==="red"?"#7f1d1d22":special==="purple"?"#2d1f4e":"#1a1a1a"}`,
    color: special==="green"?"#22c55e":special==="red"?"#ef4444":special==="purple"?"#a78bfa":"#6b7280",
    transition:"all .12s",
  });

  return (
    <>
      <style>{FONT_STYLE}</style>
      <div style={{ display:"flex", flexDirection:"column", height:"100vh", overflow:"hidden",
        background:"#080808", color:"#e5e7eb", ...sans }}>

        {/* TOP NAV */}
        <div style={{ display:"flex", alignItems:"center", gap:8, padding:"0 16px",
          height:38, borderBottom:"1px solid #111", background:"#0a0a0a", flexShrink:0 }}>
          <div style={{ ...mono, fontSize:11, display:"flex", gap:4 }}>
            <span style={{ color:"#1f1f1f" }}>nebula</span>
            <span style={{ color:"#141414" }}>›</span>
            <span style={{ color:"#1f1f1f" }}>production</span>
            <span style={{ color:"#141414" }}>›</span>
            <span style={{ color:"#9ca3af", fontWeight:500 }}>Monitor</span>
          </div>
          <div style={{ display:"flex", alignItems:"center", gap:5, padding:"2px 8px", borderRadius:4,
            background:"#0a1f10", border:"1px solid #14532d" }}>
            <span style={{ width:6,height:6,borderRadius:9999,background:"#22c55e",
              animation:isLive?"pulse-dot 1.5s ease-in-out infinite":"none", display:"block" }}/>
            <span style={{ ...mono, fontSize:8, color:"#22c55e", letterSpacing:2 }}>LIVE</span>
          </div>
          <div style={{ flex:1 }}/>
          <button onClick={()=>setIsLive(v=>!v)} style={navBtn("")}>{isLive?"⏸ Pause":"▶ Resume"}</button>
          <button style={navBtn("purple")}>↺ Replay</button>
          <button style={navBtn("red")}>✕ Cancel</button>
          <div style={{ width:1,height:16,background:"#1a1a1a" }}/>
          <button style={navBtn("")}>⚙ Settings</button>
          <button style={navBtn("green")}>⊕ New</button>
        </div>

        {/* METRICS BAR */}
        <div style={{ display:"flex", background:"#0a0a0a", borderBottom:"1px solid #111",
          height:60, flexShrink:0 }}>
          <div style={{ display:"flex", flexDirection:"column", justifyContent:"center", padding:"0 16px" }}>
            <span style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:3 }}>SUCCESS RATE</span>
            <div style={{ display:"flex", alignItems:"flex-end", gap:8 }}>
              <span style={{ ...mono, fontSize:20, fontWeight:700, color:"#22c55e", lineHeight:1 }}>98%</span>
              <Sparkline data={SPARKLINE} color="#22c55e" W={52} H={18}/>
            </div>
          </div>
          <Divider/>
          <div style={{ display:"flex", flexDirection:"column", justifyContent:"center", padding:"0 16px" }}>
            <span style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:2 }}>AVG LATENCY</span>
            <span style={{ ...mono, fontSize:20, fontWeight:700, color:"#60a5fa", lineHeight:1, marginBottom:2 }}>300ms</span>
            <div style={{ display:"flex", gap:10 }}>
              {[["P50","247ms"],["P95","891ms"],["P99","2.1s"]].map(([p,v])=>(
                <span key={p} style={{ ...mono, fontSize:7, color:"#374151" }}>{p} <span style={{ color:"#4b5563" }}>{v}</span></span>
              ))}
            </div>
          </div>
          <Divider/>
          <div style={{ display:"flex", flexDirection:"column", justifyContent:"center", padding:"0 16px" }}>
            <span style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:3 }}>THROUGHPUT</span>
            <span style={{ ...mono, fontSize:20, fontWeight:700, color:"#a78bfa", lineHeight:1 }}>115/min</span>
          </div>
          <Divider/>
          {[["Running",ct.running,"#f59e0b","running"],["Queued",ct.queued,"#6b7280","queued"],
            ["Done",ct.completed,"#22c55e","completed"],["Failed",ct.failed,"#ef4444","failed"]].map(([lbl,n,c,f])=>(
            <div key={lbl} style={{ display:"flex" }}>
              <div onClick={()=>setFilter(f)} style={{ display:"flex", flexDirection:"column",
                alignItems:"center", justifyContent:"center", padding:"0 14px", cursor:"pointer" }}
                onMouseEnter={e=>e.currentTarget.style.background="#ffffff04"}
                onMouseLeave={e=>e.currentTarget.style.background=""}>
                <span style={{ ...mono, fontSize:22, fontWeight:700, color:c, lineHeight:1 }}>{n}</span>
                <span style={{ ...mono, fontSize:7, color:"#1f1f1f", marginTop:3, letterSpacing:2 }}>{lbl.toUpperCase()}</span>
              </div>
              <Divider/>
            </div>
          ))}
          <div style={{ display:"flex", flexDirection:"column", justifyContent:"center", padding:"0 16px" }}>
            <span style={{ ...mono, fontSize:7, color:"#1f1f1f", letterSpacing:3, marginBottom:6 }}>INFRASTRUCTURE</span>
            <div style={{ display:"flex", gap:14 }}>
              {[["Workers","4/4","#22c55e"],["Cluster","1 node","#4b5563"],["Uptime","99.2%","#22c55e"]].map(([k,v,c])=>(
                <div key={k} style={{ display:"flex", flexDirection:"column", gap:2 }}>
                  <span style={{ ...mono, fontSize:11, fontWeight:600, color:c, lineHeight:1 }}>{v}</span>
                  <span style={{ ...mono, fontSize:7, color:"#1f1f1f" }}>{k}</span>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* 3-COL LAYOUT */}
        <div style={{ display:"flex", flex:1, overflow:"hidden" }}>

          {/* LEFT */}
          <div style={{ width:260, flexShrink:0, display:"flex", flexDirection:"column",
            borderRight:"1px solid #111" }}>
            <div style={{ padding:8, borderBottom:"1px solid #111", flexShrink:0 }}>
              <div style={{ display:"flex", alignItems:"center", gap:6, padding:"5px 8px",
                background:"#0a0a0a", border:"1px solid #1a1a1a", borderRadius:4 }}>
                <span style={{ color:"#1f1f1f" }}>⌕</span>
                <input value={q} onChange={e=>setQ(e.target.value)} placeholder="id, workflow..."
                  style={{ ...mono, fontSize:11, color:"#9ca3af", flex:1,
                    background:"transparent", border:"none", outline:"none" }}/>
              </div>
            </div>
            <div style={{ display:"flex", flexShrink:0, borderBottom:"1px solid #111" }}>
              {["all","running","completed","failed","queued"].map(f=>(
                <button key={f} onClick={()=>setFilter(f)} style={{
                  ...mono, fontSize:7, letterSpacing:1.5, padding:"5px 8px",
                  background:"transparent", border:"none", cursor:"pointer",
                  borderBottom:`2px solid ${filter===f?"#22c55e":"transparent"}`,
                  color: filter===f?"#22c55e":"#1f1f1f",
                  textTransform:"uppercase", flexShrink:0,
                }}>
                  {f} <span style={{ color:f==="failed"?"#ef4444":f==="running"?"#f59e0b":"#1f1f1f" }}>{ct[f]}</span>
                </button>
              ))}
            </div>
            <div style={{ flex:1, overflowY:"auto" }}>
              {list.map(ex=>{
                const isSel=sel.id===ex.id;
                return (
                  <div key={ex.id} onClick={()=>{setSel(ex);setSelectedNode(null);setTab("timeline");}}
                    style={{ padding:"10px 12px", borderBottom:"1px solid #0a0a0a", cursor:"pointer",
                      borderLeft:`2px solid ${isSel?"#22c55e":"transparent"}`,
                      background:isSel?"#0a1a0a":"transparent", transition:"all .1s" }}
                    onMouseEnter={e=>{ if(!isSel) e.currentTarget.style.background="#0c0c0c"; }}
                    onMouseLeave={e=>{ if(!isSel) e.currentTarget.style.background="transparent"; }}>
                    <div style={{ display:"flex", alignItems:"center", gap:6, marginBottom:3 }}>
                      <Dot status={ex.status} pulse={ex.status==="running"}/>
                      <span style={{ fontSize:12, fontWeight:500, color:"#e5e7eb", flex:1,
                        overflow:"hidden", textOverflow:"ellipsis", whiteSpace:"nowrap" }}>{ex.wf}</span>
                      {ex.retries>0 && <span style={{ ...mono, fontSize:8, background:"#451a03",
                        color:"#f97316", padding:"1px 4px", borderRadius:3 }}>↺{ex.retries}</span>}
                    </div>
                    <div style={{ display:"flex", alignItems:"center", gap:6, marginBottom:3 }}>
                      <span style={{ ...mono, fontSize:9, color:"#1f1f1f" }}>{ex.id}</span>
                      <Badge type={ex.trigger} scheme="trigger"/>
                    </div>
                    <div style={{ display:"flex", gap:8 }}>
                      <span style={{ ...mono, fontSize:9, color:"#1f1f1f" }}>{ex.ago}</span>
                      {ex.ms && <span style={{ ...mono, fontSize:9, fontWeight:600,
                        color:ex.ms>2000?"#f59e0b":"#2d2d2d" }}>{fmtMs(ex.ms)}</span>}
                    </div>
                    {ex.error && <div style={{ ...mono, fontSize:8, color:"#7f1d1d", marginTop:4,
                      overflow:"hidden", textOverflow:"ellipsis", whiteSpace:"nowrap" }}>
                      ✕ {ex.error.split(":")[0]}</div>}
                  </div>
                );
              })}
            </div>
          </div>

          {/* CENTER */}
          <div style={{ flex:1, display:"flex", flexDirection:"column", overflow:"hidden" }}>

            {/* Execution header */}
            <div style={{ display:"flex", alignItems:"center", gap:10, padding:"8px 16px",
              borderBottom:"1px solid #111", background:"#0a0a0a", flexShrink:0 }}>
              <div style={{ width:8,height:8,borderRadius:9999,
                background:S.STATUS[sel.status]?.c, flexShrink:0 }}/>
              <span style={{ ...mono, fontSize:12, color:"#60a5fa" }}>{sel.id}</span>
              <span style={{ ...mono, fontSize:9, color:S.STATUS[sel.status]?.c,
                background:S.STATUS[sel.status]?.bg, border:`1px solid ${S.STATUS[sel.status]?.border}`,
                padding:"1px 6px", borderRadius:3 }}>{sel.status}</span>
              <span style={{ fontSize:13, fontWeight:500, color:"#e5e7eb" }}>{sel.wf}</span>
              <div style={{ flex:1 }}/>
              {sel.ms
                ? <span style={{ ...mono, fontSize:14, fontWeight:600, color:"#60a5fa" }}>{fmtMs(sel.ms)}</span>
                : sel.status==="running" && (
                    <span style={{ ...mono, fontSize:11, color:"#f59e0b",
                      animation:"pulse-dot 1.5s ease-in-out infinite" }}>running…</span>
                  )
              }
              <div style={{ width:1,height:16,background:"#1a1a1a" }}/>
              <button style={navBtn("")}>Copy ID</button>
              <button style={navBtn("purple")}>✎ Editor</button>
            </div>

            {/* Progress bar */}
            {sel.ms && (
              <div style={{ padding:"5px 16px", borderBottom:"1px solid #0a0a0a", flexShrink:0 }}>
                <div style={{ position:"relative", height:3, borderRadius:9999, background:"#0d0d0d", overflow:"hidden" }}>
                  {steps.map(s=>(
                    <div key={s.id} style={{ position:"absolute", top:0, bottom:0, borderRadius:9999,
                      left:`${(s.start/total)*100}%`, width:`${(s.dur/total)*100}%`,
                      background:s.status==="error"?"#ef444488":s.status==="warn"?"#f59e0b88":"#22c55e55" }}/>
                  ))}
                </div>
              </div>
            )}

            {/* Tabs */}
            <div style={{ display:"flex", padding:"0 16px", borderBottom:"1px solid #111", flexShrink:0 }}>
              {TABS.map(t=>(
                <button key={t} onClick={()=>setTab(t)} style={{
                  ...mono, fontSize:10, padding:"7px 14px", letterSpacing:.5,
                  background:"transparent", border:"none", cursor:"pointer",
                  borderBottom:`2px solid ${tab===t?"#60a5fa":"transparent"}`,
                  color:tab===t?"#e5e7eb":"#374151",
                  textTransform:"capitalize", transition:"all .12s",
                }}>
                  {t}
                  {t==="events"&&events.length>0&&<span style={{ marginLeft:4, fontSize:8, color:"#1f1f1f" }}>{events.length}</span>}
                  {t==="logs"&&<span style={{ marginLeft:4, fontSize:8, color:"#1f1f1f" }}>{logs.length}</span>}
                </button>
              ))}

              {/* Node hint */}
              {tab==="timeline" && (
                <div style={{ display:"flex", alignItems:"center", marginLeft:"auto",
                  ...mono, fontSize:8, color:"#1f2937", gap:6 }}>
                  {selectedNode
                    ? <span style={{ color:"#f59e0b" }}>▼ {selectedNode.name}</span>
                    : <span>click node for details</span>
                  }
                </div>
              )}
            </div>

            {/* Content area — timeline gets node panel overlay */}
            <div style={{ flex:1, overflow:"hidden", position:"relative" }}>
              {tab==="timeline" && (
                <>
                  {/* Scrollable gantt area */}
                  <div style={{ position:"absolute", top:0, left:0, right:0,
                    bottom: selectedNode ? 270 : 0, overflowY:"auto" }}>
                    <Gantt
                      steps={steps}
                      total={total}
                      execId={sel.id}
                      selectedNode={selectedNode}
                      onSelectNode={setSelectedNode}
                    />
                  </div>

                  {/* Node detail panel slides up from bottom */}
                  {selectedNode && (
                    <NodeDetailPanel
                      node={selectedNode}
                      execId={sel.id}
                      onClose={()=>setSelectedNode(null)}
                    />
                  )}
                </>
              )}
              {tab==="logs"   && <Logs logs={logs}/>}
              {tab==="events" && <EventHist events={events}/>}
            </div>
          </div>

          {/* RIGHT */}
          <div style={{ width:212, flexShrink:0, borderLeft:"1px solid #111", overflow:"hidden" }}>
            <RightPanel exec={sel}/>
          </div>
        </div>

        {/* STATUS BAR */}
        <div style={{ display:"flex", alignItems:"center", gap:16, padding:"0 16px",
          height:22, borderTop:"1px solid #111", background:"#050505", flexShrink:0 }}>
          {[["Database","healthy"],["Workers","4/4"],["Cluster","1 node"],["Uptime","99.2%"]].map(([k,v])=>(
            <div key={k} style={{ display:"flex", alignItems:"center", gap:4 }}>
              <div style={{ width:5,height:5,borderRadius:9999,background:"#22c55e" }}/>
              <span style={{ ...mono, fontSize:8, color:"#1f1f1f" }}>{k}: <span style={{ color:"#2d2d2d" }}>{v}</span></span>
            </div>
          ))}
          <div style={{ flex:1 }}/>
          <span style={{ ...mono, fontSize:8, color:"#141414" }}>Nebula v4.1</span>
        </div>
      </div>
    </>
  );
}
