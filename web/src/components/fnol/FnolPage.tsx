import { useEffect, useState } from 'react'
import { messageOf, pasApi } from '../../api/client'

const D={surface:'#fff',border:'#D6E5F1',text:'#173042',sub:'#5F778C',bg:'#EEF6FB',teal:'#3D9CA2',orange:'#F7761F',green:'#0FA372',red:'#E25555',amber:'#F3A52E'}
const inp:React.CSSProperties={width:'100%',boxSizing:'border-box',padding:'8px 11px',borderRadius:7,border:`1px solid ${D.border}`,fontSize:13,color:D.text,background:'#fff'}
const button=(color:string):React.CSSProperties=>({padding:'9px 17px',borderRadius:8,border:0,background:color,color:'#fff',fontWeight:700,fontSize:13,cursor:'pointer'})
function Section({title,children}:{title:string;children:React.ReactNode}){return <section style={{background:D.surface,border:`1px solid ${D.border}`,borderRadius:10,overflow:'hidden'}}><div style={{padding:'9px 15px',background:'#F7FAFC',borderBottom:`1px solid ${D.border}`,fontSize:11,fontWeight:700,color:D.sub,textTransform:'uppercase'}}>{title}</div><div style={{padding:15,display:'grid',gap:11}}>{children}</div></section>}
function Field({label,children}:{label:string;children:React.ReactNode}){return <label style={{fontSize:12,fontWeight:600,color:D.sub,display:'grid',gap:5}}>{label}{children}</label>}
function badge(status:string,reviewRequired:boolean){const s=String(status||'').toUpperCase();const c=s==='PROCESSED'&&!reviewRequired?D.green:s==='REVIEW_REQUIRED'||reviewRequired?D.amber:s==='PROCESSING'?D.teal:D.sub;return <span style={{fontSize:10,fontWeight:700,padding:'3px 8px',borderRadius:10,color:c,background:`${c}18`,border:`1px solid ${c}44`}}>{s||'UNKNOWN'}</span>}

type View='list'|'submit'|'trace'
export default function FnolPage(){
 const [view,setView]=useState<View>('list')
 const [submissions,setSubmissions]=useState<any[]>([])
 const [loading,setLoading]=useState(true)
 const [error,setError]=useState('')
 const [selectedId,setSelectedId]=useState<string|null>(null)
 const [trace,setTrace]=useState<any>(null)

 async function load(){setLoading(true);setError('');try{setSubmissions(await pasApi.fnolSubmissions())}catch(e){setError(messageOf(e,'FNOL queue could not be loaded.'))}finally{setLoading(false)}}
 useEffect(()=>{load()},[])

 async function openTrace(id:string){setSelectedId(id);setView('trace');setTrace(null);try{setTrace(await pasApi.fnolTrace(id))}catch(e){setError(messageOf(e,'Trace could not be loaded.'))}}

 return <div style={{maxWidth:1200,margin:'0 auto'}}>
  <div style={{display:'flex',justifyContent:'space-between',alignItems:'flex-start',marginBottom:20}}>
   <div>
    {view!=='list'&&<button onClick={()=>{setView('list');load()}} style={{border:0,background:'none',color:D.teal,padding:0,marginBottom:8,cursor:'pointer'}}>← Back to queue</button>}
    <h2 style={{fontSize:20,color:D.text,margin:0}}>{view==='trace'?'FNOL Trace':'FNOL Intake'}</h2>
    <p style={{fontSize:12,color:D.sub,margin:'4px 0 0'}}>Claim document intake, extraction, and HubSpot ticket creation</p>
   </div>
   {view==='list'&&<button style={button(D.orange)} onClick={()=>setView('submit')}>Submit document</button>}
  </div>
  {error&&<div style={{background:'#FFF1F1',border:`1px solid ${D.red}55`,borderLeft:`4px solid ${D.red}`,color:'#9B2C2C',borderRadius:8,padding:'10px 13px',fontSize:12,marginBottom:14}}>{error}</div>}

  {view==='list'&&<section style={{background:D.surface,border:`1px solid ${D.border}`,borderRadius:12,overflow:'hidden'}}>
   <div style={{padding:'12px 16px',borderBottom:`1px solid ${D.border}`,fontSize:12,color:D.sub}}>{submissions.length} submission{submissions.length===1?'':'s'}</div>
   <div style={{overflowX:'auto'}}><table style={{width:'100%',borderCollapse:'collapse'}}>
    <thead><tr style={{background:'#F7FAFC'}}>{['File','Status','Confidence','Ticket','Submitted'].map(x=><th key={x} style={{padding:'10px 14px',textAlign:'left',fontSize:10,color:D.sub,textTransform:'uppercase',letterSpacing:'.05em'}}>{x}</th>)}</tr></thead>
    <tbody>
     {loading&&<tr><td colSpan={5} style={{padding:40,textAlign:'center',color:D.sub}}>Loading FNOL queue…</td></tr>}
     {!loading&&submissions.length===0&&<tr><td colSpan={5} style={{padding:48,textAlign:'center',color:D.sub}}>No submissions yet.</td></tr>}
     {submissions.map(s=><tr key={s.process_id} onClick={()=>openTrace(s.process_id)} style={{borderTop:`1px solid ${D.border}`,cursor:'pointer'}}>
      <td style={{padding:'12px 14px',fontSize:13,color:D.text}}>{s.original_filename||s.process_id}</td>
      <td style={{padding:'12px 14px'}}>{badge(s.status,s.human_review_required)}</td>
      <td style={{padding:'12px 14px',fontSize:13,color:D.sub}}>{s.confidence!=null?`${Math.round(s.confidence*100)}%`:'—'}</td>
      <td style={{padding:'12px 14px',fontSize:12,color:D.sub}}>{s.ticket_id||'—'}</td>
      <td style={{padding:'12px 14px',fontSize:12,color:D.sub}}>{s.created_at?new Date(s.created_at).toLocaleString():'—'}</td>
     </tr>)}
    </tbody>
   </table></div>
  </section>}

  {view==='submit'&&<FnolSubmitForm onDone={()=>{setView('list');load()}}/>}

  {view==='trace'&&selectedId&&<FnolTraceView trace={trace} processId={selectedId}/>}
 </div>
}

function FnolSubmitForm({onDone}:{onDone:()=>void}){
 const [file,setFile]=useState<File|null>(null)
 const [email,setEmail]=useState('')
 const [name,setName]=useState('')
 const [phone,setPhone]=useState('')
 const [insuranceType,setInsuranceType]=useState('auto')
 const [busy,setBusy]=useState(false)
 const [error,setError]=useState('')
 const [result,setResult]=useState<any>(null)

 async function submit(e:React.FormEvent){
  e.preventDefault()
  if(!file){setError('Select a document.');return}
  setBusy(true);setError('');setResult(null)
  try{
   const form=new FormData()
   form.append('file',file)
   form.append('email',email)
   form.append('name',name)
   form.append('phone',phone)
   form.append('insurance_type',insuranceType)
   const res=await pasApi.fnolSubmit(form)
   setResult(res)
  }catch(e){setError(messageOf(e,'FNOL submission failed.'))}finally{setBusy(false)}
 }

 if(result)return <Section title="Submitted">
  <div style={{fontSize:13,color:D.text}}>Process ID: <strong>{result.process_id}</strong></div>
  <div style={{fontSize:13,color:D.text}}>Status: {badge(result.status,result.human_review_required)}</div>
  <div style={{fontSize:13,color:D.text}}>Confidence: {Math.round(result.confidence*100)}%</div>
  <button style={button(D.orange)} onClick={onDone}>Back to queue</button>
 </Section>

 return <form onSubmit={submit} style={{maxWidth:520}}><Section title="Claim document">
  {error&&<div style={{background:'#FFF1F1',color:'#9B2C2C',padding:10,borderRadius:7,fontSize:12}}>{error}</div>}
  <Field label="Document (photo, scan, or PDF)"><input type="file" accept=".pdf,.png,.jpg,.jpeg,.bmp,.gif,.tiff,.webp" onChange={e=>setFile(e.target.files?.[0]||null)} required/></Field>
  <Field label="Insurance type"><select style={inp} value={insuranceType} onChange={e=>setInsuranceType(e.target.value)}><option value="auto">Auto</option><option value="property">Property</option><option value="life">Life</option><option value="health">Health</option><option value="marine">Marine</option></select></Field>
  <Field label="Claimant email"><input style={inp} type="email" value={email} onChange={e=>setEmail(e.target.value)} required/></Field>
  <Field label="Claimant name"><input style={inp} value={name} onChange={e=>setName(e.target.value)} required/></Field>
  <Field label="Claimant phone"><input style={inp} value={phone} onChange={e=>setPhone(e.target.value)} required/></Field>
  <button style={button(D.orange)} disabled={busy}>{busy?'Submitting…':'Submit'}</button>
 </Section></form>
}

function FnolTraceView({trace,processId}:{trace:any;processId:string}){
 if(!trace)return <div style={{padding:40,textAlign:'center',color:D.sub}}>Loading trace…</div>
 const stages=Array.isArray(trace.stages_json)?trace.stages_json:[]
 return <div style={{display:'grid',gap:14}}>
  <Section title="Submission">
   <div style={{fontSize:13,color:D.text}}>Process ID: {processId}</div>
   <div>{badge(trace.status,trace.human_review_required)}</div>
   <div style={{fontSize:13,color:D.text}}>Ticket: {trace.ticket_id||'—'}</div>
   <div style={{fontSize:13,color:D.text}}>Confidence: {trace.confidence!=null?`${Math.round(trace.confidence*100)}%`:'—'}</div>
  </Section>
  <Section title="Pipeline stages">
   {stages.length===0&&<div style={{color:D.sub,fontSize:12}}>No stage data recorded.</div>}
   {stages.map((s:any,i:number)=><div key={i} style={{borderBottom:i<stages.length-1?`1px solid ${D.bg}`:'none',paddingBottom:8,marginBottom:8}}>
    <div style={{display:'flex',justifyContent:'space-between',fontSize:12}}><strong style={{color:D.text}}>{s.name}</strong><span style={{color:s.status==='complete'?D.green:s.status==='failed'?D.red:D.sub}}>{s.status}</span></div>
    {s.detail&&<pre style={{fontSize:11,color:D.sub,margin:'4px 0 0',whiteSpace:'pre-wrap',wordBreak:'break-word'}}>{JSON.stringify(s.detail,null,2)}</pre>}
   </div>)}
  </Section>
  <Section title="Extracted fields">
   <pre style={{fontSize:11,color:D.text,margin:0,whiteSpace:'pre-wrap',wordBreak:'break-word'}}>{JSON.stringify(trace.extracted_json,null,2)}</pre>
  </Section>
  {trace.summary_json?.kb_findings&&<Section title="KB-grounded findings">
   <pre style={{fontSize:11,color:D.text,margin:0,whiteSpace:'pre-wrap',wordBreak:'break-word'}}>{JSON.stringify(trace.summary_json.kb_findings,null,2)}</pre>
  </Section>}
 </div>
}
