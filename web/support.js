(() => {
  const dayKey='terrarium-support-hidden-until',sessionKey='terrarium-support-dismissed';
  try { if (Number(localStorage.getItem(dayKey))>Date.now() || sessionStorage.getItem(sessionKey)==='1') return; } catch {}
  const modal=document.getElementById('support-dialog');
  if (!modal) return;
  const once=document.getElementById('support-close-once'),day=document.getElementById('support-close-day');
  const previousFocus=document.activeElement,previousOverflow=document.body.style.overflow;
  let dismissed=false;
  const buttons=Array.from(modal.querySelectorAll('a,button'));
  function keydown(event) {
    if (event.key==='Escape') { event.preventDefault(); close(false); }
    if (event.key==='Tab') {
      const current=buttons.indexOf(document.activeElement),next=(current+(event.shiftKey?-1:1)+buttons.length)%buttons.length;
      event.preventDefault();buttons[next].focus();
    }
  }
  function close(forDay) {
    if (dismissed) return;
    dismissed=true;
    try { if (forDay) localStorage.setItem(dayKey,String(Date.now()+86400000)); else sessionStorage.setItem(sessionKey,'1'); } catch {}
    modal.classList.add('leaving');
    document.removeEventListener('keydown',keydown);
    document.body.style.overflow=previousOverflow;
    setTimeout(()=>{modal.hidden=true;if(previousFocus instanceof HTMLElement)previousFocus.focus();},180);
  }
  once.onclick=()=>close(false);day.onclick=()=>close(true);
  modal.onclick=event=>{if(event.target===modal)close(false);};
  document.addEventListener('keydown',keydown);
  const image=modal.querySelector('img');image.src=image.dataset.src;
  document.body.style.overflow='hidden';modal.hidden=false;once.focus();
})();
