import sys
# Tile 824x824 at (100,100) inside a 1024 canvas; letter coordinates in [0,1] of the tile.
N = int(sys.argv[1]) if len(sys.argv)>1 else 13
apex=(0.5,0.19); footL=(0.20,0.81); footR=(0.80,0.81)
t=0.115            # horizontal leg thickness
cb=(0.585,0.655)   # crossbar y-range
x0,x1=0.20,0.80
pitch=(x1-x0)/(N-1)
w=pitch*0.62
def legx(y, foot):  # leg centre x at height y
    return apex[0] + (foot[0]-apex[0])*(y-apex[1])/(foot[1]-apex[1])
def inside(x,y):
    if y<apex[1]-0.02 or y>footL[1]: return False
    for foot in (footL,footR):
        if abs(x-legx(y,foot))<=t/2: return True
    if cb[0]<=y<=cb[1] and legx(y,footL)<=x<=legx(y,footR): return True
    return False
bars=[]
for i in range(N):
    x=x0+i*pitch
    ys=[apex[1]-0.03+k*0.002 for k in range(int((footL[1]-apex[1]+0.05)/0.002))]
    run=None
    for y in ys+[9]:
        if y!=9 and inside(x,y):
            run = run or [y,y]; run[1]=y
        elif run:
            if run[1]-run[0] > 0.01: bars.append((x,run[0],run[1]))
            run=None
T=824; O=100
rects=[]
for x,ya,yb in bars:
    ya=max(ya,apex[1]-0.01); h=(yb-ya)
    rects.append(f'<rect x="{O+(x-w/2)*T:.1f}" y="{O+(ya-w/2+0.004)*T:.1f}" width="{w*T:.1f}" height="{(h+w-0.008)*T:.1f}" rx="{w*T/2:.1f}"/>')
svg=f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <!-- Assunta app icon: vertical sound-wave bars that together draw the letter "A" -->
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#4F46E5"/>
      <stop offset="1" stop-color="#8B5CF6"/>
    </linearGradient>
  </defs>
  <rect x="100" y="100" width="824" height="824" rx="190" fill="url(#bg)"/>
  <g fill="#FFFFFF">
    {chr(10).join("    "+r for r in rects).strip()}
  </g>
</svg>
'''
open('icon.svg','w').write(svg)
print(len(rects),'bars')
