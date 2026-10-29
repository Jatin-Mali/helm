// Run with node --test tests/demo/dashboard.test.cjs. Executes the shipped dashboard script.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
function dashboard(fetch = async()=>{}) {
  const listeners = {};
  const nodes = new Map();
  function node(id) {
    if (!nodes.has(id)) nodes.set(id, {textContent:'',disabled:false,open:false,value:'',
      addEventListener(){}, querySelectorAll(){return [];}, showModal(){this.open=true;}});
    return nodes.get(id);
  }
  const document = {activeElement:{tagName:'BODY',closest(){return null;}},
    getElementById:node, querySelectorAll(){return [];},
    addEventListener(name,fn){listeners[name]=fn;}};
  const context = vm.createContext({document, window:{fetch,confirm:()=>true},
    Headers, Response, setInterval(){}, setTimeout, clearTimeout, console, Date});
  const html = readFileSync('helm-cli/src/web/dashboard.html','utf8');
  const script = html.match(/<script>([\s\S]*?)<\/script>/)[1].replace(/authenticate\(\);\s*$/, '');
  vm.runInContext(script,context);
  vm.runInContext("var messages=[]; toast=message=>messages.push(message)",context);
  return {context,document,node,listeners,run:code=>vm.runInContext(code,context)};
}
test('Enter on native controls retains native activation',()=>{
  const d=dashboard();
  d.document.activeElement={tagName:'BUTTON',closest(){return this;}};
  d.run("state.focusedFindingId='incident'; selectFinding=()=>{throw Error('queue hijacked control')}");
  let prevented=false;
  d.listeners.keydown({key:'Enter',preventDefault(){prevented=true;}});
  assert.equal(prevented,false);
});
test('modal pairing blocks dashboard shortcuts',()=>{
  const d=dashboard();d.node('login-dialog').open=true;
  d.run("refreshSnapshot=()=>{throw Error('shortcut escaped modal')}");
  d.listeners.keydown({key:'F5',preventDefault(){}});
});
test('reviewed actions reject duplicate requests until completion',async()=>{
  const d=dashboard();
  d.run("var calls=0; var finish; postJson=()=>{calls++;return new Promise(resolve=>finish=resolve)}");
  const first=d.run("reviewedAction('/action',{unit:'fixture.service',action:'stop'})");
  const second=d.run("reviewedAction('/action',{unit:'fixture.service',action:'stop'})");
  assert.equal(d.run('calls'),1,'a pending action must own its request across renders');
  d.run("finish({ok:true})");
  await first;assert.match((await second).error,/progress/);
});
test('plan request shows loading before the first response',async()=>{
  const d=dashboard();
  d.run("state.activeTab='services'; state.findings=[{id:'f',fingerprint:'fp'}];state.selectedFindingId='f'; var finish;postJson=()=>new Promise(resolve=>finish=resolve)");
  const pending=d.run('requestPlan()');
  assert.equal(d.run('state.findings[0].planStatus'),'loading');
  d.run("finish({plan_status:'failed',plan_error:'fixture'})");await pending;
});

test('expired authentication opens pairing and returns an actionable error', async()=>{
  const d=dashboard(async()=>new Response('',{status:401}));
  const response=await d.run("authenticatedFetch('/api/snapshot')");
  assert.equal(d.node('login-dialog').open,true);
  d.context.response=response;
  const body=await d.run('jsonOrError(response)');
  assert.match(body.error,/Session expired/);
});
test('non-JSON server failures become an error object',async()=>{
  const d=dashboard();
  const body=await d.run("jsonOrError(new Response('offline',{status:503}))");
  assert.match(body.error,/HTTP 503/);
});
test('snapshot replacement restores focus to the same contextual control',()=>{
  for(const dataset of [{serviceUnit:'fixture.service',serviceAction:'stop'},{fingerprint:'fixture-finding'},{planRetry:''}]) {
    const d=dashboard();let focused=false;
    d.document.activeElement={tagName:'BUTTON',dataset};
    d.document.querySelectorAll=()=>[{dataset,focus(){focused=true;}}];
    d.run('rememberFocus()()');
    assert.equal(focused,true,JSON.stringify(dataset));
  }
});
