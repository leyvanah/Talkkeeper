// This is an interface illustration. Only the reading views switch.
(() => {
  for (const [selector,attribute,prefix] of [['[data-preview-layout]','previewLayout','preview-'],['[data-preview-panel]','previewPanel','preview-']]) {
    const tabs=[...document.querySelectorAll(selector)];
    function show(chosen,focus=false){for(const tab of tabs){const active=tab===chosen;tab.setAttribute('aria-selected',String(active));tab.tabIndex=active?0:-1;document.getElementById(prefix+tab.dataset[attribute]).hidden=!active;}if(focus)chosen.focus();}
    if(attribute==='previewLayout'&&matchMedia('(max-width:900px)').matches)show(tabs.find(tab=>tab.dataset.previewLayout==='chat'));
    tabs.forEach((tab,index)=>{tab.addEventListener('click',()=>show(tab));tab.addEventListener('keydown',event=>{let next;if(event.key==='ArrowRight')next=(index+1)%tabs.length;if(event.key==='ArrowLeft')next=(index+tabs.length-1)%tabs.length;if(event.key==='Home')next=0;if(event.key==='End')next=tabs.length-1;if(next!==undefined){event.preventDefault();show(tabs[next],true);}});});
  }
})();
