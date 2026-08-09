/**
 * enumerate.c — 多联骨牌枚举引擎（OpenMP 并行版）
 *
 * 并行策略:
 *   - 形状级并行: cur_list → flat array → omp parallel for dynamic
 *   - orient_hs: 无锁读 + critical 写, 预分配不扩容
 *   - 每线程独立 ChunkList → 代末 O(1) 拼接
 *   - 计数器: reduction / atomic
 *
 * 单线程优化（保留）:
 *   - 全局方向集跳过规范化
 *   - __builtin_ctzll 位迭代
 *   - 分块数组 + 包围盒预判
 */

#include "enumerate.h"
#include "hashset.h"
#include "timer.h"
#include "chunklist.h"

#ifdef _OPENMP
#include <omp.h>
#endif

/* ================================================================
 * __builtin_ctzll 位迭代
 * ================================================================ */

static void mask_to_cells(mask_t mask, int w, int h,
                          int cells_r[], int cells_c[], int *count) {
    (void)w; (void)h;
    *count = 0;
    mask_t m = mask;
    while (m) {
        int bit = __builtin_ctzll(m);
        cells_r[*count] = bit >> STRIDE_SHIFT;
        cells_c[*count] = bit & (STRIDE - 1);
        (*count)++;
        m &= m - 1;
    }
}

static void mask_get_extent(mask_t mask, int *w, int *h) {
    *w = 0; *h = 0;
    mask_t m = mask;
    while (m) {
        int bit = __builtin_ctzll(m);
        int r = bit >> STRIDE_SHIFT, c = bit & (STRIDE - 1);
        if (c + 1 > *w) *w = c + 1;
        if (r + 1 > *h) *h = r + 1;
        m &= m - 1;
    }
}

/* ================================================================
 * 规范化 + 旋转（位迭代版）
 * ================================================================ */

static mask_t normalize_mask(mask_t cells, int w, int h,
                              int *out_w, int *out_h) {
    (void)w; (void)h;
    int min_r = MAX_N, max_r = -1, min_c = MAX_N, max_c = -1;
    mask_t m = cells;
    while (m) {
        int bit = __builtin_ctzll(m);
        int r = bit >> STRIDE_SHIFT, c = bit & (STRIDE - 1);
        if (r < min_r) min_r = r;
        if (r > max_r) max_r = r;
        if (c < min_c) min_c = c;
        if (c > max_c) max_c = c;
        m &= m - 1;
    }
    if (min_r > max_r) { *out_w = 0; *out_h = 0; return 0; }
    mask_t result = 0;
    m = cells;
    while (m) {
        int bit = __builtin_ctzll(m);
        int r = (bit >> STRIDE_SHIFT) - min_r;
        int c = (bit & (STRIDE - 1)) - min_c;
        result |= 1ULL << (r * STRIDE + c);
        m &= m - 1;
    }
    *out_w = max_c - min_c + 1;
    *out_h = max_r - min_r + 1;
    return result;
}

static mask_t rotate90(mask_t cells, int w, int h, int *out_w, int *out_h) {
    mask_t result = 0;
    mask_t m = cells;
    while (m) {
        int bit = __builtin_ctzll(m);
        int r = bit >> STRIDE_SHIFT, c = bit & (STRIDE - 1);
        result |= 1ULL << (c * STRIDE + (h - 1 - r));
        m &= m - 1;
    }
    *out_w = h; *out_h = w;
    return result;
}

static void compute_orientations(mask_t cells, int w, int h,
                                  mask_t orients[4],
                                  mask_t *canonical,
                                  int *can_w, int *can_h) {
    mask_t best = UINT64_MAX;
    int bw = 0, bh = 0;
    mask_t cur = cells;
    int cw = w, ch = h;
    for (int rot = 0; rot < 4; rot++) {
        int nw, nh;
        mask_t norm = normalize_mask(cur, cw, ch, &nw, &nh);
        orients[rot] = norm;
        if (norm < best) { best = norm; bw = nw; bh = nh; }
        cur = rotate90(cur, cw, ch, &cw, &ch);
    }
    *canonical = best;
    *can_w = bw; *can_h = bh;
}

/* ================================================================
 * 洞检测 — Flood Fill
 * ================================================================ */

static bool poly_has_hole(mask_t cells, int w, int h) {
    if (w < 3 || h < 3) return false;  /* 窄盒不可能有洞 */
    int gh = h + 2, gw = w + 2;
    bool occupied[GRID_PAD][GRID_PAD];
    bool visited[GRID_PAD][GRID_PAD];
    memset(occupied, 0, sizeof(occupied));
    memset(visited, 0, sizeof(visited));
    mask_t m = cells;
    while (m) {
        int bit = __builtin_ctzll(m);
        occupied[(bit >> STRIDE_SHIFT) + 1][(bit & (STRIDE - 1)) + 1] = true;
        m &= m - 1;
    }
    int qr[GRID_PAD * GRID_PAD], qc[GRID_PAD * GRID_PAD];
    int head = 0, tail = 0;
    qr[tail] = 0; qc[tail] = 0; tail++;
    visited[0][0] = true;
    static const int dr[] = {-1, 1, 0, 0};
    static const int dc[] = {0, 0, -1, 1};
    while (head < tail) {
        int r = qr[head], c = qc[head]; head++;
        for (int d = 0; d < 4; d++) {
            int nr = r + dr[d], nc = c + dc[d];
            if (nr >= 0 && nr < gh && nc >= 0 && nc < gw)
                if (!visited[nr][nc] && !occupied[nr][nc]) {
                    visited[nr][nc] = true;
                    qr[tail] = nr; qc[tail] = nc; tail++;
                }
        }
    }
    for (int r = 1; r <= h; r++)
        for (int c = 1; c <= w; c++)
            if (!occupied[r][c] && !visited[r][c]) return true;
    return false;
}

/* ================================================================
 * 主枚举 — OpenMP 并行版
 * ================================================================ */

RoomCount *enumerate_all(int max_n, int *out_count) {
    *out_count = max_n;
    RoomCount *results = (RoomCount *)calloc((size_t)max_n, sizeof(RoomCount));
    if (!results) return NULL;
    for (int i = 0; i < max_n; i++) results[i].n = i + 1;

    /* 方向哈希集: 预分配不扩容 (关键: 并行期无锁读安全)
       n≤5 → 8M, n≥6 → 256M (后续可按需扩容但需串行化) */
    int orient_cap = (max_n <= 5) ? (1 << 23) : (1 << 28);
    HashSet *orient_hs = hs_create(orient_cap);
    if (!orient_hs) { free(results); return NULL; }

    /* 单格起始 */
    mask_t start = 1ULL;
    hs_insert(orient_hs, start);
    hs_insert(orient_hs, start); /* 4 方向全相同 — 存 4 份简化逻辑 */
    hs_insert(orient_hs, start);
    hs_insert(orient_hs, start);

    ChunkList cur_list, next_list;
    cl_init(&cur_list); cl_init(&next_list);
    cl_add(&cur_list, start);

    for (int n = 1; n <= max_n; n++) {
        results[n - 1].total++; results[n - 1].no_hole++;
    }

    int total_gen = 1, hole_all = 0, fast_skip = 0;
    int max_cells = max_n * max_n;

    for (int size = 1; size < max_cells; size++) {

        /* ============================================
         * 阶段1: 扁平化 cur_list → 数组
         * ============================================ */
        int n_shapes = cur_list.total;
        mask_t *flat = (mask_t *)malloc((size_t)n_shapes * sizeof(mask_t));
        if (!flat) goto oom;
        {
            int idx = 0;
            for (Chunk *ch = cur_list.head; ch; ch = ch->next)
                for (int pi = 0; pi < ch->count; pi++)
                    flat[idx++] = ch->data[pi];
        }

        /* ============================================
         * 阶段2: 每线程独立输出列表
         * ============================================ */
        int n_threads = 1;
#ifdef _OPENMP
        n_threads = omp_get_max_threads();
#endif
        ChunkList *tl = (ChunkList *)calloc((size_t)n_threads,
                                             sizeof(ChunkList));
        if (!tl) { free(flat); goto oom; }
        for (int t = 0; t < n_threads; t++) cl_init(&tl[t]);

        /* 线程本地计数 (reduction) */
        int local_fast = 0, local_hole = 0, local_gen = 0;

        /* ============================================
         * 阶段3: 并行枚举
         * if(n_shapes<500) → 自动串行（避免并行开销压倒小批次）
         * ============================================ */
#pragma omp parallel for if(n_shapes >= 500) schedule(dynamic, 16) \
    reduction(+:local_fast, local_hole, local_gen)
        for (int sidx = 0; sidx < n_shapes; sidx++) {
            int tid = 0;
#ifdef _OPENMP
            tid = omp_get_thread_num();
#endif
            mask_t pmask = flat[sidx];

            /* 线程局部分配 */
            int cells_r[MAX_CELLS], cells_c[MAX_CELLS];

            int pw, ph;
            mask_get_extent(pmask, &pw, &ph);
            int cell_count;
            mask_to_cells(pmask, pw, ph, cells_r, cells_c, &cell_count);

            bool box_at_max = (pw == max_n && ph == max_n);

            /* ---------- 前沿计算 ---------- */
            bool occ[GRID_PAD][GRID_PAD];
            bool in_front[GRID_PAD][GRID_PAD];
            memset(occ, 0, sizeof(occ));
            memset(in_front, 0, sizeof(in_front));
            for (int i = 0; i < cell_count; i++)
                occ[cells_r[i] + 1][cells_c[i] + 1] = true;

            int fr[256], fc[256], fcount = 0;
            static const int dr[] = {-1, 1, 0, 0};
            static const int dc[] = {0, 0, -1, 1};

            for (int i = 0; i < cell_count; i++) {
                int r = cells_r[i], c = cells_c[i];
                for (int d = 0; d < 4; d++) {
                    int nr = r + dr[d], nc = c + dc[d];
                    if (occ[nr + 1][nc + 1]) continue;
                    if (in_front[nr + 1][nc + 1]) continue;
                    if (box_at_max) {
                        if (nr < 0 || nr >= ph || nc < 0 || nc >= pw)
                            continue;
                    } else {
                        int nw = pw, nh = ph;
                        if (nc < 0) nw++; else if (nc >= pw) nw = nc + 1;
                        if (nr < 0) nh++; else if (nr >= ph) nh = nr + 1;
                        if (nw > max_n || nh > max_n) continue;
                    }
                    in_front[nr + 1][nc + 1] = true;
                    fr[fcount] = nr; fc[fcount] = nc; fcount++;
                }
            }

            /* ---------- 逐个前沿扩展 ---------- */
            for (int fi = 0; fi < fcount; fi++) {
                int nr = fr[fi], nc = fc[fi];

                /* GROW */
                int sr = (nr < 0), sc = (nc < 0);
                mask_t new_mask = 0;
                for (int r = 0; r < ph; r++) {
                    mask_t row = (pmask >> (r * STRIDE)) &
                                 ((1ULL << pw) - 1);
                    new_mask |= (row << sc) << ((r + sr) * STRIDE);
                }
                new_mask |= 1ULL << ((nr + sr) * STRIDE + (nc + sc));
                int raw_w = pw, raw_h = ph;
                if (nc < 0) raw_w++; else if (nc >= pw) raw_w = nc + 1;
                if (nr < 0) raw_h++; else if (nr >= ph) raw_h = nr + 1;

                /* [A] 无锁查方向集 (resize 不会发生 → 安全) */
                bool hit = hs_contains(orient_hs, new_mask);

                if (hit) { local_fast++; continue; }

                /* 新形状: 计算 4 方向 */
                mask_t orients[4], canonical;
                int can_w, can_h;
                compute_orientations(new_mask, raw_w, raw_h,
                                      orients, &canonical,
                                      &can_w, &can_h);

                /* [临界区] 原子插入 4 方向 */
                bool inserted;
#pragma omp critical(orient_insert)
                {
                    /* 交叉检查: 别的线程可能已插入 */
                    if (hs_contains(orient_hs, canonical)) {
                        inserted = false;
                    } else {
                        hs_insert(orient_hs, orients[0]);
                        hs_insert(orient_hs, orients[1]);
                        hs_insert(orient_hs, orients[2]);
                        hs_insert(orient_hs, orients[3]);
                        inserted = true;
                    }
                }

                if (!inserted) continue; /* 其他线程先插入了 */
                local_gen++;

                /* 添加到线程本地列表 */
                cl_add(&tl[tid], canonical);

                /* 洞检测 */
                bool hole = poly_has_hole(canonical, can_w, can_h);
                if (hole) local_hole++;

                int md = (can_w > can_h) ? can_w : can_h;
                for (int n = md; n <= max_n; n++) {
#pragma omp atomic
                    results[n - 1].total++;
                    if (hole) {
#pragma omp atomic
                        results[n - 1].has_hole++;
                    } else {
#pragma omp atomic
                        results[n - 1].no_hole++;
                    }
                }
            }
        }

        /* ============================================
         * 阶段4: 合并线程本地列表 + 还原计数
         * ============================================ */
        fast_skip += local_fast;
        hole_all  += local_hole;
        total_gen += local_gen;

        for (int t = 0; t < n_threads; t++)
            cl_merge(&next_list, &tl[t]);

        free(tl);
        free(flat);

        fprintf(stderr,
                "  [枚举] 格=%2d 本代=%d 生成=%d 累计=%d "
                "洞=%d 快跳=%d\n",
                size, cur_list.total, next_list.total, total_gen,
                hole_all, fast_skip);

        if (next_list.total == 0) break;
        cl_free(&cur_list);
        cl_swap(&cur_list, &next_list);
        cl_init(&next_list);
    }

    cl_free(&cur_list); cl_free(&next_list);
    hs_free(orient_hs);
    return results;

oom:
    cl_free(&cur_list); cl_free(&next_list);
    free(results);
    hs_free(orient_hs);
    return NULL;
}
