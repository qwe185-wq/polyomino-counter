/**
 * enumerate.c — 增强计时版（临界区 + 等锁计时）
 */

#include "enumerate.h"
#include "hashset.h"
#include "timer.h"
#include "chunklist.h"
#ifdef _OPENMP
#include <omp.h>
#endif

/* ================================================================
 * __builtin_ctzll
 * ================================================================ */
static void mask_to_cells(mask_t mask, int w, int h,
                          int cells_r[], int cells_c[], int *count) {
    (void)w; (void)h; *count = 0; mask_t m = mask;
    while (m) { int bit = __builtin_ctzll(m);
        cells_r[*count]=bit>>STRIDE_SHIFT; cells_c[*count]=bit&(STRIDE-1);
        (*count)++; m&=m-1; }
}
static void mask_get_extent(mask_t mask, int *w, int *h) {
    *w=0; *h=0; mask_t m=mask;
    while (m) { int bit=__builtin_ctzll(m);
        int r=bit>>STRIDE_SHIFT, c=bit&(STRIDE-1);
        if(c+1>*w)*w=c+1; if(r+1>*h)*h=r+1; m&=m-1; }
}
static mask_t normalize_mask(mask_t cells, int w, int h,
                              int *out_w, int *out_h) {
    (void)w;(void)h; int min_r=MAX_N,max_r=-1,min_c=MAX_N,max_c=-1;
    mask_t m=cells;
    while(m){int bit=__builtin_ctzll(m);
        int r=bit>>STRIDE_SHIFT,c=bit&(STRIDE-1);
        if(r<min_r)min_r=r;if(r>max_r)max_r=r;
        if(c<min_c)min_c=c;if(c>max_c)max_c=c;m&=m-1;}
    if(min_r>max_r){*out_w=0;*out_h=0;return 0;}
    mask_t result=0;m=cells;
    while(m){int bit=__builtin_ctzll(m);
        result|=1ULL<<(((bit>>STRIDE_SHIFT)-min_r)*STRIDE
                       +(bit&(STRIDE-1))-min_c);m&=m-1;}
    *out_w=max_c-min_c+1;*out_h=max_r-min_r+1;return result;
}
static mask_t rotate90(mask_t cells, int w, int h, int *out_w, int *out_h) {
    mask_t result=0;mask_t m=cells;
    while(m){int bit=__builtin_ctzll(m);
        result|=1ULL<<((bit&(STRIDE-1))*STRIDE
                       +(h-1-(bit>>STRIDE_SHIFT)));m&=m-1;}
    *out_w=h;*out_h=w;return result;
}
static void compute_orientations(mask_t cells, int w, int h,
                                  mask_t orients[4], mask_t *canonical,
                                  int *can_w, int *can_h) {
    mask_t best=UINT64_MAX;int bw=0,bh=0;
    mask_t cur=cells;int cw=w,ch=h;
    for(int rot=0;rot<4;rot++){int nw,nh;
        mask_t norm=normalize_mask(cur,cw,ch,&nw,&nh);
        orients[rot]=norm;if(norm<best){best=norm;bw=nw;bh=nh;}
        cur=rotate90(cur,cw,ch,&cw,&ch);}
    *canonical=best;*can_w=bw;*can_h=bh;
}
static bool poly_has_hole(mask_t cells, int w, int h) {
    if(w<3||h<3)return false;int gh=h+2,gw=w+2;
    bool occ[GRID_PAD][GRID_PAD],vis[GRID_PAD][GRID_PAD];
    memset(occ,0,sizeof(occ));memset(vis,0,sizeof(vis));
    mask_t m=cells;while(m){int bit=__builtin_ctzll(m);
        occ[(bit>>STRIDE_SHIFT)+1][(bit&(STRIDE-1))+1]=true;m&=m-1;}
    int qr[GRID_PAD*GRID_PAD],qc[GRID_PAD*GRID_PAD];
    int hd=0,tl=0;qr[tl]=0;qc[tl]=0;tl++;vis[0][0]=true;
    static const int dr[]={-1,1,0,0},dc[]={0,0,-1,1};
    while(hd<tl){int r=qr[hd],c=qc[hd];hd++;
        for(int d=0;d<4;d++){int nr=r+dr[d],nc=c+dc[d];
            if(nr>=0&&nr<gh&&nc>=0&&nc<gw)
                if(!vis[nr][nc]&&!occ[nr][nc])
                {vis[nr][nc]=true;qr[tl]=nr;qc[tl]=nc;tl++;}}}
    for(int r=1;r<=h;r++)for(int c=1;c<=w;c++)
        if(!occ[r][c]&&!vis[r][c])return true;
    return false;
}

/* ================================================================
 * 主枚举
 * ================================================================ */
RoomCount *enumerate_all(int max_n, int *out_count) {
    *out_count=max_n;
    RoomCount *results=(RoomCount*)calloc((size_t)max_n,sizeof(RoomCount));
    if(!results)return NULL;
    for(int i=0;i<max_n;i++)results[i].n=i+1;

    /* 方向集: 大容量预分配防扩容
       n≤5→8M, n≥6→256M (可能不够，但先尝试) */
    int cap=(max_n<=5)?(1<<23):(1<<28);
    HashSet *orient_hs=hs_create(cap);
    if(!orient_hs){free(results);return NULL;}

    mask_t start=1ULL;
    for(int i=0;i<4;i++)hs_insert(orient_hs,start);

    ChunkList cur_list,next_list;
    cl_init(&cur_list);cl_init(&next_list);
    cl_add(&cur_list,start);

    for(int n=1;n<=max_n;n++)
    {results[n-1].total++;results[n-1].no_hole++;}

    int total_gen=1,hole_all=0,fast_skip=0,max_cells=max_n*max_n;

    for(int size=1;size<max_cells;size++){
        double t_gen=timer_gen_start();
        int n_shapes=cur_list.total;
        mask_t *flat=(mask_t*)malloc((size_t)n_shapes*sizeof(mask_t));
        if(!flat)goto oom;
        {int idx=0;for(Chunk *ch=cur_list.head;ch;ch=ch->next)
            for(int pi=0;pi<ch->count;pi++)flat[idx++]=ch->data[pi];}

        int n_threads=1;
#ifdef _OPENMP
        n_threads=omp_get_max_threads();
#endif
        ChunkList *tl=(ChunkList*)calloc((size_t)n_threads,sizeof(ChunkList));
        if(!tl){free(flat);goto oom;}
        for(int t=0;t<n_threads;t++)cl_init(&tl[t]);

        int local_fast=0,local_hole=0,local_gen=0;
        double local_wait=0.0;

#pragma omp parallel for if(n_shapes>=500) schedule(dynamic,16) \
    reduction(+:local_fast,local_hole,local_gen,local_wait)
        for(int sidx=0;sidx<n_shapes;sidx++){
            int tid=0;
#ifdef _OPENMP
            tid=omp_get_thread_num();
#endif
            mask_t pmask=flat[sidx];
            int cells_r[MAX_CELLS],cells_c[MAX_CELLS];
            int pw,ph;mask_get_extent(pmask,&pw,&ph);
            int cc;mask_to_cells(pmask,pw,ph,cells_r,cells_c,&cc);
            bool box_at_max=(pw==max_n&&ph==max_n);

            bool occ[GRID_PAD][GRID_PAD],inf[GRID_PAD][GRID_PAD];
            memset(occ,0,sizeof(occ));memset(inf,0,sizeof(inf));
            for(int i=0;i<cc;i++)occ[cells_r[i]+1][cells_c[i]+1]=true;
            int fr[256],fc[256],fcount=0;
            static const int dr[]={-1,1,0,0},dc[]={0,0,-1,1};
            for(int i=0;i<cc;i++){int r=cells_r[i],c=cells_c[i];
                for(int d=0;d<4;d++){int nr=r+dr[d],nc=c+dc[d];
                    if(occ[nr+1][nc+1])continue;
                    if(inf[nr+1][nc+1])continue;
                    if(box_at_max){if(nr<0||nr>=ph||nc<0||nc>=pw)continue;}
                    else{int nw=pw,nh=ph;
                        if(nc<0)nw++;else if(nc>=pw)nw=nc+1;
                        if(nr<0)nh++;else if(nr>=ph)nh=nr+1;
                        if(nw>max_n||nh>max_n)continue;}
                    inf[nr+1][nc+1]=true;fr[fcount]=nr;fc[fcount]=nc;fcount++;}}

            for(int fi=0;fi<fcount;fi++){int nr=fr[fi],nc=fc[fi];
                int sr=(nr<0),sc=(nc<0);mask_t new_mask=0;
                for(int r=0;r<ph;r++){mask_t row=(pmask>>(r*STRIDE))
                    &((1ULL<<pw)-1);
                    new_mask|=(row<<sc)<<((r+sr)*STRIDE);}
                new_mask|=1ULL<<((nr+sr)*STRIDE+(nc+sc));
                int raw_w=pw,raw_h=ph;
                if(nc<0)raw_w++;else if(nc>=pw)raw_w=nc+1;
                if(nr<0)raw_h++;else if(nr>=ph)raw_h=nr+1;

                TIMER_START(TIMER_ORIENT_LOOKUP);
                bool hit=hs_contains(orient_hs,new_mask);
                TIMER_STOP(TIMER_ORIENT_LOOKUP);
                if(hit){local_fast++;continue;}

                TIMER_START(TIMER_CANONICAL);
                mask_t orients[4],canonical;int can_w,can_h;
                compute_orientations(new_mask,raw_w,raw_h,
                                      orients,&canonical,&can_w,&can_h);
                TIMER_STOP(TIMER_CANONICAL);

                TIMER_START(TIMER_HS_INSERT);
                double tw=TIMER_START_WAIT();
                bool inserted;
#pragma omp critical(orient_insert)
                {local_wait+=TIMER_NOW()-tw;
                    if(hs_contains(orient_hs,canonical))inserted=false;
                    else{hs_insert(orient_hs,orients[0]);
                        hs_insert(orient_hs,orients[1]);
                        hs_insert(orient_hs,orients[2]);
                        hs_insert(orient_hs,orients[3]);inserted=true;}}
                TIMER_STOP(TIMER_HS_INSERT);
                if(!inserted)continue;

                local_gen++;cl_add(&tl[tid],canonical);

                TIMER_START(TIMER_HOLE);
                bool hole=poly_has_hole(canonical,can_w,can_h);
                TIMER_STOP(TIMER_HOLE);
                if(hole)local_hole++;
                int md=(can_w>can_h)?can_w:can_h;
                for(int n=md;n<=max_n;n++){
#pragma omp atomic
                    results[n-1].total++;
                    if(hole){
#pragma omp atomic
                        results[n-1].has_hole++;
                    }else{
#pragma omp atomic
                        results[n-1].no_hole++;
                    }
                }
            }
        }

        fast_skip+=local_fast;hole_all+=local_hole;total_gen+=local_gen;

        TIMER_START(TIMER_MERGE);
        for(int t=0;t<n_threads;t++)cl_merge(&next_list,&tl[t]);
        TIMER_STOP(TIMER_MERGE);

        free(tl);free(flat);

        fprintf(stderr,"  [枚举] 格=%2d 本代=%d 生成=%d 累计=%d "
                "洞=%d 快跳=%d\n",
                size,cur_list.total,next_list.total,total_gen,
                hole_all,fast_skip);
        timer_gen_print(size,t_gen,(int)(local_wait*1000),
                        n_shapes,next_list.total);

        if(next_list.total==0)break;
        cl_free(&cur_list);cl_swap(&cur_list,&next_list);cl_init(&next_list);
    }

    cl_free(&cur_list);cl_free(&next_list);
    hs_free(orient_hs);return results;

oom:
    cl_free(&cur_list);cl_free(&next_list);
    free(results);hs_free(orient_hs);return NULL;
}
