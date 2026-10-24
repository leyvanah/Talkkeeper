(() => {
  const svg = document.getElementById('sound-feather');
  if (!svg) return;
  const container = svg.parentElement;
  const group = document.getElementById('feather-strands');
  const silhouette = document.getElementById('logo-feather');
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  const count = 54;
  const spacing = 650/(count-1);
  const columnX = i => 455+i*spacing;
  const paths = [];
  const lerp = (a,b,t) => a+(b-a)*t;
  const ease = t => t*t*(3-2*t);
  const tailCount = 20;
  const leaderIndex = count-10; // Tenth bar when counting from right to left.
  const leaderX = 455+leaderIndex/(count-1)*650;
  const angularSpeed = Math.PI*2/3;
  const ramp = (time,start,end) => ease(Math.max(0,Math.min(1,(time-start)/(end-start))));
  // Fit the existing feather contour to its 22 fixed column positions.
  // This keeps every bar on the initial lattice instead of moving bars sideways.
  document.getElementById('feather-scan-mask').setAttribute('transform',`translate(${columnX(20)} 0) scale(${spacing/13.5} 1) translate(-674 0)`);
  // Slice the original logo outline into vertical sections.
  const outlines=[...silhouette.querySelectorAll('path')].map(path=>{
    const length=path.getTotalLength();
    return Array.from({length:513},(_,i)=>{
      const point=path.getPointAtLength(length*i/512);
      return [250+point.x*3.3,-40+point.y*3.3];
    });
  });
  function sections(x) {
    const intervals=[];
    for(const polygon of outlines) {
      const crossings=[];
      for(let j=1;j<polygon.length;j++) {
        const a=polygon[j-1],b=polygon[j];
        if((a[0]<=x&&b[0]>x)||(b[0]<=x&&a[0]>x))crossings.push(lerp(a[1],b[1],(x-a[0])/(b[0]-a[0])));
      }
      crossings.sort((a,b)=>a-b);
      for(let j=0;j+1<crossings.length;j+=2)intervals.push([crossings[j],crossings[j+1]]);
    }
    return intervals.sort((a,b)=>a[0]-b[0]);
  }
  const slices=Array.from({length:22},(_,i)=>sections(674+i*13.5));
  for (let i=0;i<count;i++) {
    const path=document.createElementNS('http://www.w3.org/2000/svg','path');
    group.append(path);paths.push(path);
  }
  function render(seconds,still=false) {
    const phase=seconds%30;
    // Give each column time to follow the contour, then let the feather breathe
    // before returning to the wave. The waveform itself keeps its original clock.
    const passage=still?1:ramp(phase,3,12);
    const route=still?1:ramp(phase,3,9)*(1-ramp(phase,18,27));
    const gather=route;
    container.classList.toggle('is-feather',passage>.95&&route>.8);
    silhouette.setAttribute('opacity','0');
    group.setAttribute('opacity','1');
    const front=lerp(leaderX,560,passage);
    const rendered=[];
    for(let i=0;i<count;i++) {
      const t=i/(count-1);
      const x=455+t*650;
      const envelope=Math.pow(Math.sin(Math.PI*t),.65);
      // Increasing phase sends the crests towards the left.
      const amplitude=12+envelope*(38+100*(.5+.5*Math.sin(i*.58+seconds*angularSpeed)));
      const px=x;
      let top=382-amplitude,bottom=382+amplitude;
      let opacity=.35+.55*envelope,width=5.5;
      let shapeAmount=0;
      let bands=null;
      if(i<tailCount) {
        // The remaining waveform lives at the lower left, attached to the quill.
        const u=i/(tailCount-1);
        const tailEnvelope=Math.sin(Math.PI*u);
        const tailHeight=10+tailEnvelope*(28+40*(.5+.5*Math.sin(i*.58+seconds*angularSpeed)));
        top=lerp(top,534-tailHeight,gather);
        bottom=lerp(bottom,534+tailHeight,gather);
        width=lerp(5.5,5,gather);
        opacity=lerp(opacity,.4+.5*tailEnvelope,gather);
      } else if(i<42) {
        const targetX=674+(i-tailCount)*13.5;
        // A broad travelling transition prevents individual pieces rushing
        // downwards when the scan reaches their column.
        shapeAmount=route*ramp(targetX,front-100,front+100);
        const contours=slices[i-tailCount];
        if(contours.length&&shapeAmount>0) {
          const breathe=still?1:.97+.03*Math.sin(i*.58+seconds*angularSpeed);
          const localJoin=i<=24;
          const total=localJoin?contours.reduce((sum,[a,b])=>sum+b-a,0):1;
          let accumulated=0;
          bands=contours.map(([a,b])=>{
            const center=(a+b)/2;
            let start=top,end=bottom;
            if(localJoin) {
              // Split one bar into touching pieces, instead of sending several
              // full-height copies towards the shaft and nib independently.
              start=lerp(top,bottom,accumulated/total);
              accumulated+=b-a;
              end=lerp(top,bottom,accumulated/total);
            }
            return [lerp(start,center+(a-center)*breathe,shapeAmount),lerp(end,center+(b-center)*breathe,shapeAmount)];
          });
        }
        // Keep the original thin strokes; there is no solid-ink stage.
        width=5.5;
        opacity=lerp(opacity,.95,shapeAmount);
      } else {
        // The tenth bar leads the incoming wave back down to its original center.
        // Neighbours follow with a slight delay rather than dropping as one block.
        const followDelay=Math.abs(i-leaderIndex)*.09;
        const returnToCenter=still?1:ramp(phase,4.8+followDelay,7.8+followDelay);
        const bend=route*(1-returnToCenter)*ease(Math.max(0,Math.min(1,(1105-x)/145)));
        const center=lerp(382,155,bend);
        const halfHeight=lerp(amplitude,16,bend);
        top=center-halfHeight;bottom=center+halfHeight;
      }
      const segments=bands||[[top,bottom]];
      rendered.push({x:px,segments});
      paths[i].setAttribute('d',segments.map(([a,b])=>`M${px.toFixed(2)} ${a.toFixed(2)}L${px.toFixed(2)} ${b.toFixed(2)}`).join(''));
      paths[i].setAttribute('stroke-width',width.toFixed(2));
      paths[i].setAttribute('opacity',opacity.toFixed(2));
      if(shapeAmount>.999&&!(i>=20&&i<=24))paths[i].setAttribute('clip-path','url(#feather-scan-mask)');
      else paths[i].removeAttribute('clip-path');
    }
    // Preserve the accepted vertical transitions at the two joins.
    function bridge(first,last,leftIndex,rightIndex,wholeBar) {
      if(route===0)return;
      const left=rendered[leftIndex],right=rendered[rightIndex];
      const leftBand=wholeBar?[left.segments[0][0],left.segments[left.segments.length-1][1]]:left.segments[0];
      const rightBand=wholeBar?[right.segments[0][0],right.segments[right.segments.length-1][1]]:right.segments[0];
      for(let i=first;i<=last;i++) {
        const t=ease((i-leftIndex)/(rightIndex-leftIndex));
        const original=rendered[i];
        const target=[lerp(leftBand[0],rightBand[0],t),lerp(leftBand[1],rightBand[1],t)];
        const base=wholeBar?[original.segments[0][0],original.segments[original.segments.length-1][1]]:original.segments[0];
        const connected=[lerp(base[0],target[0],route),lerp(base[1],target[1],route)];
        // At the left join keep the lower nib sections exactly as they were.
        const segments=wholeBar?[connected]:[connected,...original.segments.slice(1)];
        rendered[i].segments=segments;
        paths[i].setAttribute('d',segments.map(([a,b])=>`M${original.x.toFixed(2)} ${a.toFixed(2)}L${original.x.toFixed(2)} ${b.toFixed(2)}`).join(''));
        paths[i].removeAttribute('clip-path');
      }
    }
    bridge(41,43,40,44,true); // Up to the eleventh bar; the tenth and right wave stay intact.
    bridge(20,22,19,23,false); // Smooth the little curl without changing the left wave or nib.
  }
  let frame=0,elapsed=0,previous=0,visible=true;
  function tick(now) {
    if(previous) elapsed+=(now-previous)/1000;
    previous=now;render(elapsed);frame=requestAnimationFrame(tick);
  }
  function sync() {
    cancelAnimationFrame(frame);previous=0;
    if(reduced.matches){render(0,true);return;}
    if(visible&&!document.hidden)frame=requestAnimationFrame(tick);
  }
  new IntersectionObserver(entries=>{visible=entries[0].isIntersecting;sync();},{threshold:0}).observe(svg);
  document.addEventListener('visibilitychange',sync);
  reduced.addEventListener('change',sync);
  render(0,reduced.matches);
})();
