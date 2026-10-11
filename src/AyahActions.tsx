import { BookOpen, Headphones, ChevronDown, X } from 'lucide-react';
import { useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { autoUpdate, flip, shift, size, offset, useFloating, FloatingFocusManager, FloatingOverlay } from '@floating-ui/react';
export type ListeningScope = 'ayah'|'page'|'surah'|'quran'|'whole-surah'|'whole-page';
export function AyahActions({ x,y,title,arabic,onClose,onStudy,onListen }: {x:number;y:number;title:string;arabic:boolean;onClose:()=>void;onStudy:()=>void;onListen:(scope:ListeningScope)=>void}) {
  const t=(en:string,ar:string)=>arabic?ar:en;
  const firstAction = useRef<HTMLButtonElement>(null);
  const [listenChoicesOpen, setListenChoicesOpen] = useState(false);
  const listenChoicesId = useId();
  const { refs, floatingStyles, context } = useFloating({
    open: true, onOpenChange: open => { if (!open) onClose(); }, strategy: 'fixed', placement: 'bottom-start',
    whileElementsMounted: autoUpdate,
    middleware: [offset(6), flip({ padding: 12 }), shift({ padding: 12, crossAxis: true }), size({ padding: 12,
      apply: ({ availableHeight, elements }) => { elements.floating.style.maxHeight = Math.max(80, availableHeight) + 'px'; } })],
  });
  useLayoutEffect(() => {
    refs.setPositionReference({ getBoundingClientRect: () => new DOMRect(x, y, 0, 0) });
  }, [refs.setPositionReference, x, y]);
  const keyDown=(event:KeyboardEvent<HTMLDivElement>)=>{
    event.stopPropagation();
    if(event.key==='Escape')onClose();
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      const controls = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('button,summary')).filter(el => el.getClientRects().length);
      const index = controls.indexOf(document.activeElement as HTMLElement);
      event.preventDefault(); controls[(index + (event.key === 'ArrowUp' ? -1 : 1) + controls.length) % controls.length]?.focus();
    }
  };
  return <FloatingOverlay className="ayahActionsBackdrop" onClick={onClose} onContextMenu={e=>{e.preventDefault();onClose();}}>
    <FloatingFocusManager context={context} initialFocus={firstAction}><div ref={refs.setFloating} className="ayahActionsMenu" role="dialog" aria-label={t('Ayah actions','خيارات الآية')} dir={arabic?'rtl':'ltr'} style={floatingStyles} onClick={e=>e.stopPropagation()} onKeyDown={keyDown}>
      <header><strong>{title}</strong><button aria-label={t('Close','إغلاق')} onClick={onClose}><X size={16}/></button></header>
      <button ref={firstAction} className="ayahPrimaryAction" onClick={onStudy}><BookOpen size={19}/><span>{t('Tafsir','التفسير')}<small>{t('Read the explanation','اقرأ تفسير الآية')}</small></span></button>
      <div className="ayahListeningGroup">
        <div className="ayahListenHeading">
          <button className="ayahPrimaryAction" onClick={()=>onListen('surah')}><Headphones size={19}/><span>{t('Listen from here','استمع من هنا')}<small>{t('Continue to the end of this surah','حتى نهاية هذه السورة')}</small></span></button>
          <button className="ayahListenExpand" aria-label={t('More listening options','خيارات استماع أخرى')} aria-expanded={listenChoicesOpen} aria-controls={listenChoicesId} onClick={()=>setListenChoicesOpen(open=>!open)}><ChevronDown size={17}/></button>
        </div>
        <div id={listenChoicesId} className="ayahListenChoices" hidden={!listenChoicesOpen} role="group" aria-label={t('Listening range','نطاق الاستماع')}>
          {([['ayah','This ayah only','هذه الآية فقط'],['page','To end of page','حتى نهاية الصفحة'],['whole-surah','Listen to whole surah','استمع إلى السورة كاملة'],['quran','To end of Quran','حتى نهاية القرآن']] as const).map(([scope,en,ar])=><button key={scope} onClick={()=>onListen(scope)}>{t(en,ar)}</button>)}
        </div>
      </div>
    </div></FloatingFocusManager>
  </FloatingOverlay>;
}
