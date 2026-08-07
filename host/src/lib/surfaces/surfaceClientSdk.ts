// Builds the document served into a sandboxed surface frame: the app's own
// HTML bundle, with a host-authored CSP `<meta>` and the in-frame bridge SDK
// (`window.appHost`) injected into <head>. The SDK is the ONLY way frame code
// reaches the host — there is no Tauri API, no kernel handle, no filesystem.
//
// The SDK talks to the host exclusively via `window.parent.postMessage`, and
// buffers outbound traffic until the host's `init` arrives, so app code may
// call `ready()` / read ops eagerly at top level.

import type { SurfaceUiBundle } from "$lib/api";
import { SURFACE_BRIDGE_PROTOCOL, SURFACE_BRIDGE_VERSION } from "./surfaceBridgeProtocol";

/// The in-frame SDK source. Interpolates the protocol constants so the frame
/// and host can never drift on the wire contract. Pure string — it runs inside
/// the sandbox, not in the host bundle.
export function buildClientSdk(): string {
  return `(function(){
  var PROTOCOL=${JSON.stringify(SURFACE_BRIDGE_PROTOCOL)},VERSION=${SURFACE_BRIDGE_VERSION};
  var instanceId=null,seq=0,pending={},eventCbs=[],extEventCbs=[],readyRequested=false,preInit=[],preStates=[];
  var ctx={appId:null,surface:null,capabilities:[],configSchema:null,config:{},extensionContext:{},hostContext:{},theme:null,variables:{}};
  var appliedVariables=[];
  var lastHeight=-1,heightWatching=false;
  function send(msg){ try{ window.parent.postMessage(msg,"*"); }catch(_){} }
  function reportHeight(){
    if(instanceId===null) return;
    var doc=document.documentElement,body=document.body;
    // Use the <html> box height (shrinks to content), NOT scrollHeight, which is
    // clamped up to the iframe viewport height and would just echo the frame's
    // current size back. body.scrollHeight guards against content that overflows
    // the html box.
    var h=doc?doc.getBoundingClientRect().height:0;
    if(body&&body.scrollHeight>h) h=body.scrollHeight;
    h=Math.ceil(h);
    if(h<=0||h===lastHeight) return;
    lastHeight=h;
    send({protocol:PROTOCOL,v:VERSION,type:"resize",instanceId:instanceId,height:h});
  }
  function watchHeight(){
    if(heightWatching) return; heightWatching=true;
    reportHeight();
    try{
      if(typeof ResizeObserver!=="undefined"){
        var ro=new ResizeObserver(function(){ reportHeight(); });
        if(document.documentElement) ro.observe(document.documentElement);
        if(document.body) ro.observe(document.body);
      }
    }catch(_){}
    window.addEventListener("load",reportHeight);
  }
  function flush(){
    if(instanceId===null) return;
    if(readyRequested){ readyRequested=false; send({protocol:PROTOCOL,v:VERSION,type:"ready",instanceId:instanceId}); }
    var queued=preInit; preInit=[];
    queued.forEach(function(q){ dispatchOp(q.op,q.resolve,q.reject,q.onProgress); });
    var states=preStates; preStates=[];
    states.forEach(function(payload){ sendState(payload); });
    watchHeight();
  }
  function sendState(payload){
    send({protocol:PROTOCOL,v:VERSION,type:"extension-state",instanceId:instanceId,payload:payload});
  }
  function dispatchOp(op,resolve,reject,onProgress){
    var id=++seq; pending[id]={resolve:resolve,reject:reject,onProgress:onProgress};
    send({protocol:PROTOCOL,v:VERSION,type:"request",instanceId:instanceId,requestId:id,op:op});
  }
  function requestOp(op,onProgress){
    return new Promise(function(resolve,reject){
      if(instanceId===null){ preInit.push({op:op,resolve:resolve,reject:reject,onProgress:onProgress}); return; }
      dispatchOp(op,resolve,reject,onProgress);
    });
  }
  function applyTheme(theme,variables){
    if(theme!=="light"&&theme!=="dark") return;
    ctx.theme=theme;
    var root=document.documentElement;
    if(root){
      root.setAttribute("data-theme",theme); root.style.colorScheme=theme;
      appliedVariables.forEach(function(name){ root.style.removeProperty(name); });
      appliedVariables=[];
      if(variables&&typeof variables==="object") Object.keys(variables).forEach(function(name){
        var value=variables[name];
        if(/^--(?:color|app-color)-[a-z0-9-]+$/.test(name)&&typeof value==="string"&&value.length<=80){
          root.style.setProperty(name,value); appliedVariables.push(name);
        }
      });
    }
    ctx.variables=(variables&&typeof variables==="object")?variables:{};
    if(window.appHost){ window.appHost.theme=theme; window.appHost.variables=ctx.variables; }
  }
  window.addEventListener("message",function(e){
    // Host messages arrive from the parent window only. Reject anything from a
    // sibling app frame, which is otherwise reachable and could forge "host"
    // input (init/extensionContext, invoke replies) that the frame trusts.
    if(e.source!==window.parent) return;
    var m=e.data;
    if(!m||m.protocol!==PROTOCOL||m.v!==VERSION) return;
    if(m.type==="init"){
      instanceId=m.instanceId; ctx.appId=m.appId; ctx.surface=m.surface;
       ctx.capabilities=m.capabilities||[]; ctx.configSchema=m.configSchema; ctx.config=m.config||{}; ctx.extensionContext=m.extensionContext||{}; ctx.hostContext=m.hostContext||{};
      applyTheme(m.theme,m.variables);
      var h=window.appHost;
      h.appId=ctx.appId; h.surface=ctx.surface; h.capabilities=ctx.capabilities;
       h.configSchema=ctx.configSchema; h.config=ctx.config; h.extensionContext=ctx.extensionContext; h.hostContext=ctx.hostContext;
      flush();
      if(typeof h._oninit==="function"){ try{ h._oninit(ctx); }catch(_){} }
    } else if(m.type==="response"){
      var p=pending[m.requestId]; if(!p) return; delete pending[m.requestId];
      if(m.ok) p.resolve(m.result); else p.reject(new Error(m.error||"request failed"));
    } else if(m.type==="progress"){
      var active=pending[m.requestId];
      if(active&&typeof active.onProgress==="function"){ try{ active.onProgress(m.value); }catch(_){} }
    } else if(m.type==="event"){
      eventCbs.forEach(function(cb){ try{ cb(); }catch(_){} });
    } else if(m.type==="extension-event"){
      var payload=(m.payload&&typeof m.payload==="object")?m.payload:{};
      extEventCbs.forEach(function(cb){ try{ cb(payload); }catch(_){} });
    } else if(m.type==="theme"){
      applyTheme(m.theme,m.variables); reportHeight();
    }
  });
  window.appHost={
    ready:function(){ readyRequested=true; flush(); },
    reportError:function(msg){ if(instanceId!==null) send({protocol:PROTOCOL,v:VERSION,type:"error",instanceId:instanceId,message:String(msg)}); },
    invoke:function(capability,input,goal,onProgress){ return requestOp({kind:"invoke",capability:capability,input:input||{},data_scope:{kind:"none"},goal:goal||""},onProgress); },
    invokeScoped:function(capability,input,dataScope,goal,onProgress){ return requestOp({kind:"invoke",capability:capability,input:input||{},data_scope:dataScope,goal:goal||""},onProgress); },
    cancelRun:function(runId){ return requestOp({kind:"cancel-run",runId:String(runId)}); },
    getConfig:function(){ return requestOp({kind:"get-config"}); },
    updateConfig:function(config){ return requestOp({kind:"update-config",config:config||{}}); },
    getState:function(key){ return requestOp({kind:"get-state",key:String(key)}); },
    putState:function(key,expectedRevision,value){ return requestOp({kind:"put-state",key:String(key),expectedRevision:expectedRevision,value:value===null?null:(value||{})}); },
     data:{v1:{
      get:function(collection,id){ return requestOp({kind:"data-v1",request:{kind:"get",collection:String(collection),id:String(id)}}); },
      list:function(collection,query){ return requestOp({kind:"data-v1",request:{kind:"list",collection:String(collection),query:query||{}}}); },
      create:function(collection,value){ return requestOp({kind:"data-v1",request:{kind:"create",collection:String(collection),value:value||{}}}); },
      replace:function(collection,id,expectedRevision,value){ return requestOp({kind:"data-v1",request:{kind:"replace",collection:String(collection),id:String(id),expectedRevision:expectedRevision,value:value||{}}}); },
      delete:function(collection,id,expectedRevision){ return requestOp({kind:"data-v1",request:{kind:"delete",collection:String(collection),id:String(id),expectedRevision:expectedRevision}}); },
       transaction:function(operations){ return requestOp({kind:"data-v1",request:{kind:"transaction",operations:Array.isArray(operations)?operations:[]}}); }
      },v2:{
        readSnapshot:function(request){ return requestOp({kind:"data-v2",request:{kind:"read-snapshot",...(request&&request.expectedGeneration===undefined?{}:{expectedGeneration:request&&request.expectedGeneration}),reads:Array.isArray(request&&request.reads)?request.reads:[]}}); },
        get:function(request){ return requestOp({kind:"data-v2",request:{kind:"get",collection:String(request.collection),id:String(request.id),...(request.expectedGeneration===undefined?{}:{expectedGeneration:request.expectedGeneration})}}); },
        list:function(request){ return requestOp({kind:"data-v2",request:{kind:"list",collection:String(request.collection),...(request.query===undefined?{}:{query:request.query}),...(request.expectedGeneration===undefined?{}:{expectedGeneration:request.expectedGeneration})}}); },
        getDocument:function(request){ return requestOp({kind:"data-v2",request:{kind:"get-document",collection:String(request.collection),id:String(request.id),offset:request.offset,length:request.length,...(request.expectedGeneration===undefined?{}:{expectedGeneration:request.expectedGeneration})}}); },
        listDocuments:function(request){ return requestOp({kind:"data-v2",request:{kind:"list-documents",collection:String(request.collection),...(request.after===undefined?{}:{after:request.after}),...(request.limit===undefined?{}:{limit:request.limit}),...(request.expectedGeneration===undefined?{}:{expectedGeneration:request.expectedGeneration})}}); },
        create:function(request){ return requestOp({kind:"data-v2",request:{kind:"create",mutationId:String(request.mutationId),expectedGeneration:request.expectedGeneration,collection:String(request.collection),value:request.value||{}}}); },
        replace:function(request){ return requestOp({kind:"data-v2",request:{kind:"replace",mutationId:String(request.mutationId),expectedGeneration:request.expectedGeneration,collection:String(request.collection),id:String(request.id),expectedRevision:request.expectedRevision,value:request.value||{}}}); },
        delete:function(request){ return requestOp({kind:"data-v2",request:{kind:"delete",mutationId:String(request.mutationId),expectedGeneration:request.expectedGeneration,collection:String(request.collection),id:String(request.id),expectedRevision:request.expectedRevision}}); },
         beginBatch:function(request){ return requestOp({kind:"data-v2",request:{kind:"begin-batch",mutationId:String(request.mutationId),expectedGeneration:request.expectedGeneration,operations:Array.isArray(request.operations)?request.operations:[],documents:Array.isArray(request.documents)?request.documents:[]}}); },
         appendBatchOperations:function(request){ return requestOp({kind:"data-v2",request:{kind:"append-batch-operations",mutationId:String(request.mutationId),batchId:String(request.batchId),operations:Array.isArray(request.operations)?request.operations:[]}}); },
         appendDocumentChunk:function(request){ return requestOp({kind:"data-v2",request:{kind:"append-document-chunk",mutationId:String(request.mutationId),batchId:String(request.batchId),documentId:String(request.documentId),chunkIndex:request.chunkIndex,contentBase64:String(request.contentBase64)}}); },
        commitBatch:function(request){ return requestOp({kind:"data-v2",request:{kind:"commit-batch",mutationId:String(request.mutationId),batchId:String(request.batchId)}}); },
        abortBatch:function(request){ return requestOp({kind:"data-v2",request:{kind:"abort-batch",mutationId:String(request.mutationId),batchId:String(request.batchId)}}); }
     }},
    listArtifacts:function(){ return requestOp({kind:"list-artifacts"}); },
    listEvents:function(){ return requestOp({kind:"list-events"}); },
    onEvent:function(cb){ if(typeof cb==="function") eventCbs.push(cb); },
    onExtensionEvent:function(cb){ if(typeof cb==="function") extEventCbs.push(cb); },
    publishExtensionState:function(payload){
      var value=(payload&&typeof payload==="object")?payload:{};
      if(instanceId===null){ preStates.push(value); return; }
      sendState(value);
    },
    onInit:function(cb){ window.appHost._oninit=cb; if(instanceId!==null&&typeof cb==="function"){ try{ cb(ctx); }catch(_){} } },
    appId:null,surface:null,capabilities:[],configSchema:null,config:{},extensionContext:{},hostContext:{},theme:null,variables:{}
  };
  window.addEventListener("error",function(ev){ window.appHost.reportError((ev&&ev.message)||"script error"); });
})();`;
}

/// Inject the host-authored CSP `<meta>` and the bridge SDK into the bundle's
/// document. The CSP meta is placed first inside <head> so it governs
/// everything that follows; the SDK script runs before the app's own scripts
/// so `window.appHost` always exists.
export function buildSurfaceSrcdoc(bundle: SurfaceUiBundle): string {
  // frame-ancestors is ignored in meta-delivered CSP and only produces a
  // browser warning. Embedding isolation is enforced by the iframe sandbox.
  const metaCsp = bundle.csp
    .split(";")
    .map((directive) => directive.trim())
    .filter((directive) => directive !== "" && !directive.toLowerCase().startsWith("frame-ancestors"))
    .join("; ");
  const head = [
    `<meta http-equiv="Content-Security-Policy" content="${escapeAttr(metaCsp)}">`,
    `<script>${buildClientSdk()}</script>`,
  ].join("\n");

  const html = bundle.html;
  const headOpen = html.match(/<head[^>]*>/i);
  if (headOpen && headOpen.index !== undefined) {
    const at = headOpen.index + headOpen[0].length;
    return html.slice(0, at) + "\n" + head + html.slice(at);
  }
  const htmlOpen = html.match(/<html[^>]*>/i);
  if (htmlOpen && htmlOpen.index !== undefined) {
    const at = htmlOpen.index + htmlOpen[0].length;
    return html.slice(0, at) + `<head>${head}</head>` + html.slice(at);
  }
  // No document scaffold: wrap the fragment in one we control.
  return `<!doctype html><html><head>${head}</head><body>${html}</body></html>`;
}

function escapeAttr(value: string): string {
  // The attribute is double-quoted, so single quotes (ubiquitous in CSP:
  // 'none', 'self', 'unsafe-inline') are left readable. Everything that could
  // break out of the attribute or start markup is escaped.
  return value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}
