import { useEffect, useState } from 'react'
import { BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, ResponsiveContainer } from 'recharts'
import { pasApi } from '../../api/client'

const D = { bg:'#EEF6FB', surface:'#FFFFFF', border:'#D6E5F1', text:'#173042', textSub:'#5F778C', teal:'#3D9CA2', orange:'#F7761F', purple:'#102D7B', green:'#0FA372' }

type Metrics = { quotes:number; quoted:number; bound:number; policies:number; writtenPremium:number }
function rows(body:any, key:string) { return Array.isArray(body) ? body : (body?.[key] || body?.items || []) }
function Card({label,value,accent}:{label:string;value:string|number;accent:string}) { return <div style={{background:D.surface,border:`1px solid ${D.border}`,borderLeft:`4px solid ${accent}`,borderRadius:10,padding:'14px 16px'}}><div style={{fontSize:11,color:D.textSub,textTransform:'uppercase',fontWeight:700,marginBottom:6}}>{label}</div><div style={{fontSize:28,fontWeight:800,color:D.text}}>{value}</div><div style={{fontSize:10,color:D.textSub,marginTop:5}}>Live native PAS data</div></div> }

export default function ReportsPage() {
  const [m,setM]=useState<Metrics>({quotes:0,quoted:0,bound:0,policies:0,writtenPremium:0})
  const [available,setAvailable]=useState(true)
  useEffect(()=>{ let cancelled=false; Promise.all([
    pasApi.quotes(),
    pasApi.policies(),
  ]).then(([qb,pb])=>{ if(cancelled)return; const q=rows(qb,'quotes'); const p=rows(pb,'policies'); setM({quotes:q.length,quoted:q.filter((x:any)=>String(x.status).toUpperCase()==='QUOTED').length,bound:q.filter((x:any)=>String(x.status).toUpperCase()==='BOUND').length,policies:p.length,writtenPremium:p.reduce((s:number,x:any)=>s+Number(x.premiumAmount||x.totalPremium||x.premium||0),0)}); setAvailable(true)}).catch(()=>{if(!cancelled)setAvailable(false)}); return()=>{cancelled=true} },[])
  const chart=[{name:'Quotes',value:m.quotes},{name:'Quoted',value:m.quoted},{name:'Bound',value:m.bound},{name:'Policies',value:m.policies}]
  return <div style={{maxWidth:1150,margin:'0 auto'}}>
    <div style={{marginBottom:18}}><h2 style={{fontSize:22,fontWeight:800,color:D.text,margin:'0 0 4px'}}>PAS Reports & Analytics</h2><p style={{fontSize:12,color:D.textSub,margin:0}}>Quotes, policy lifecycle, and written premium from verified native Rust endpoints.</p></div>
    {!available&&<div style={{background:'#FFF6ED',border:'1px solid #FFC391',borderLeft:`4px solid ${D.orange}`,color:'#98440E',borderRadius:8,padding:'11px 14px',fontSize:12,marginBottom:16}}>Native PAS reporting data is unavailable. No fallback or sample values are being displayed.</div>}
    <div style={{display:'grid',gridTemplateColumns:'repeat(4,1fr)',gap:12,marginBottom:16}}><Card label="All Quotes" value={m.quotes} accent={D.teal}/><Card label="Quoted" value={m.quoted} accent={D.purple}/><Card label="Bound" value={m.bound} accent={D.orange}/><Card label="Written Premium" value={available?m.writtenPremium.toLocaleString('en-US',{style:'currency',currency:'USD'}):'—'} accent={D.green}/></div>
    <div style={{display:'grid',gridTemplateColumns:'2fr 1fr',gap:16}}>
      <section style={{background:D.surface,border:`1px solid ${D.border}`,borderRadius:12,padding:'16px 18px'}}><div style={{fontSize:14,fontWeight:700,color:D.text,marginBottom:12}}>Policy lifecycle</div><div style={{height:280}}><ResponsiveContainer width="100%" height="100%"><BarChart data={chart}><CartesianGrid strokeDasharray="3 3" stroke={D.border}/><XAxis dataKey="name" tick={{fontSize:11,fill:D.textSub}}/><YAxis allowDecimals={false} tick={{fontSize:11,fill:D.textSub}}/><Tooltip/><Bar dataKey="value" fill={D.teal} radius={[5,5,0,0]}/></BarChart></ResponsiveContainer></div></section>
      <section style={{background:D.surface,border:`1px solid ${D.border}`,borderRadius:12,padding:'16px 18px'}}><div style={{fontSize:14,fontWeight:700,color:D.text,marginBottom:14}}>Exports</div><div style={{background:D.bg,borderRadius:8,padding:14,fontSize:12,color:D.textSub,lineHeight:1.6}}>BDX export remains visible but unavailable until a verified native backend route exists.</div><button disabled title="Backend integration pending" style={{width:'100%',marginTop:12,padding:10,borderRadius:8,border:`1px solid ${D.border}`,background:'#F5F8FA',color:'#8AA0B1',fontWeight:700,cursor:'not-allowed'}}>Export BDX — Backend integration pending</button></section>
    </div>
  </div>
}
