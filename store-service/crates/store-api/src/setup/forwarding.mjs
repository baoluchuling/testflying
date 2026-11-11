export function bindForwarding(request,getSession) {
  const $=(id)=>document.getElementById(id);
  let settings; let epoch=0; let busy=false;
  function render() {
    for(const platform of ["ios","android"]) {
      const enabled=settings?.[platform];
      $(platform+"ForwardingState").textContent=enabled===undefined?"状态未知":enabled?"转发已开启":"转发已暂停";
      const button=$(platform+"ForwardingToggle");button.disabled=busy||enabled===undefined;
      button.textContent=enabled?"暂停转发":"开启转发";
      button.setAttribute("aria-pressed",String(Boolean(enabled)));
    }
    $("forwardingRefresh").disabled=busy;
  }
  async function perform(platform) {
    const active=getSession();if(!active||busy)return;
    if(platform&&!settings)return;
    const current=++epoch;busy=true;render();$("forwardingStatus").textContent="正在更新转发状态…";
    try {
      const result=await request(active,platform?`forwarding/${platform}`:"forwarding",platform?{method:"PUT",body:{enabled:!settings[platform]}}:{});
      if(current!==epoch||active.signal.aborted)return;
      settings=result;$("forwardingStatus").textContent=platform?"设置已保存，立即影响后续请求。":"转发状态已刷新。";
    }catch(error){if(current===epoch&&!active.signal.aborted){settings=undefined;$("forwardingStatus").textContent=error.message||"读取转发状态失败，请刷新重试。";}}
    finally {if(current===epoch){busy=false;render();}}
  }
  for(const platform of ["ios","android"])$(platform+"ForwardingToggle").addEventListener("click",()=>perform(platform));
  $("forwardingRefresh").addEventListener("click",()=>perform());
  return {reset(){epoch++;busy=false;settings=undefined;$("forwardingWorkspace").hidden=true;$("forwardingStatus").textContent="";render();},show(){ $("forwardingWorkspace").hidden=false;perform();}};
}
