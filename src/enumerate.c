/**
 * enumerate.c — 多联骨牌枚举引擎（方向集跳过 + 位扫描版）
 *
 * 核心优化:
 *   A. 全局方向哈希集 — 每个新形状存全部 4 个归一化方向
 *      重复候选: 1 次哈希查找 = 跳过规范化 (省 75% 归一化)
 *   B. __builtin_ctzll — 只迭代置位 bit
 *   C. 分块数组 + 包围盒预判
 */

#include "enumerate.h"
#include "hashset.h"
#include "timer.h"
#include "chunklist.h"

/* ================================================================
 * __builtin_ctzll 位迭代 — 只扫描置位 bit
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

/**
 * 计算全部 4 归一化方向 + canonical（最小值）= 一次扫描出所有
 *
 * 关键：4 个方向存入 orient_hs（全局方向哈希集），
 * canonical 存入 canonical_hs + next_list。
 */
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
 * 主枚举
 *
 * [优化A] 全局方向哈希集 (orient_hs):
 *   存每个形状的全部 4 个归一化旋转掩码。
 *   候选的 GROW raw mask（已归一化）直接查 orient_hs:
 *     → 命中 → 跳过规范化 (省 ~75%)
 *     → 未命中 → 计算 4 方向 → 全部插入 orient_hs
 *   canonical_hs 仅用于 cross-check 罕见哈希碰撞。
 *
 *   复杂度:
 *     重复: 1×归一化(免费,GROW已做) + O(1)哈希查找
 *     新形状: 4×归一化(=当前 canonical) + 4×哈希插入
 *     总归一化: 0.9×(0) + 0.1×(4) = 0.4× 原来
 * ================================================================ */

RoomCount *enumerate_all(int max_n, int *out_count) {
    *out_count = max_n;
    RoomCount *results = (RoomCount *)calloc((size_t)max_n, sizeof(RoomCount));
    if (!results) return NULL;
    for (int i = 0; i < max_n; i++) results[i].n = i + 1;

    /* 方向哈希集: 存全部 4 归一化旋转（~2M 条目 for n=5） */
    HashSet *orient_hs = hs_create(1 << 22);    /* 初始 4M 容量 */
    /* 规范化形式集合: 仅用于罕见碰撞交叉验证 */
    HashSet *canon_hs  = hs_create(1 << 20);

    if (!orient_hs || !canon_hs) {
        free(results); hs_free(orient_hs); hs_free(canon_hs);
        return NULL;
    }

    ChunkList cur_list, next_list;
    cl_init(&cur_list); cl_init(&next_list);

    int cells_r[MAX_CELLS], cells_c[MAX_CELLS];

    /* 单格起始 */
    mask_t start = 1ULL;
    hs_insert(orient_hs, start);
    hs_insert(canon_hs, start);
    cl_add(&cur_list, start);

    for (int n = 1; n <= max_n; n++) {
        results[n - 1].total++; results[n - 1].no_hole++;
    }

    int total_gen = 1, hole_all = 0, fast_skip = 0;
    int max_cells = max_n * max_n;

    for (int size = 1; size < max_cells; size++) {

        for (Chunk *ch = cur_list.head; ch; ch = ch->next) {
            for (int pi = 0; pi < ch->count; pi++) {
                mask_t pmask = ch->data[pi];

                TIMER_START(TIMER_EXTRACT);
                int pw, ph;
                mask_get_extent(pmask, &pw, &ph);
                int cell_count;
                mask_to_cells(pmask, pw, ph, cells_r, cells_c, &cell_count);
                TIMER_STOP(TIMER_EXTRACT);

                bool box_at_max = (pw == max_n && ph == max_n);

                TIMER_START(TIMER_FRONTIER);
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
                TIMER_STOP(TIMER_FRONTIER);

                for (int fi = 0; fi < fcount; fi++) {
                    int nr = fr[fi], nc = fc[fi];

                    TIMER_START(TIMER_GROW);
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
                    TIMER_STOP(TIMER_GROW);

                    /* ======================================
                     * [优化A] 方向集快速去重
                     *
                     * GROW 产生的 new_mask 已归一化(0,0)。
                     * 查 orient_hs:
                     *  命中 → 跳过规范化 (99% 重复)
                     *  未命中 → 计算 4 方向 + canonical
                     * ====================================== */
                    TIMER_START(TIMER_CANONICAL);

                    TIMER_START(TIMER_INVARIANT);
                    bool orient_hit = hs_contains(orient_hs, new_mask);
                    TIMER_STOP(TIMER_INVARIANT);

                    if (orient_hit) {
                        TIMER_STOP(TIMER_CANONICAL);
                        fast_skip++;
                        continue;
                    }

                    /* 新形状 → 计算全部 4 方向 */
                    mask_t orients[4], canonical;
                    int can_w, can_h;
                    compute_orientations(new_mask, raw_w, raw_h,
                                          orients, &canonical,
                                          &can_w, &can_h);
                    TIMER_STOP(TIMER_CANONICAL);

                    /* 交叉验证: canonical 是否真唯一
                       (方向集无碰撞，但 canonical 可能有) */
                    TIMER_START(TIMER_HASHSET);
                    bool is_new = hs_insert(canon_hs, canonical);
                    TIMER_STOP(TIMER_HASHSET);
                    if (!is_new) continue;  /* 极罕见哈希碰撞 */

                    /* 4 方向全部插入 */
                    TIMER_START(TIMER_HASHSET);
                    hs_insert(orient_hs, orients[0]);
                    hs_insert(orient_hs, orients[1]);
                    hs_insert(orient_hs, orients[2]);
                    hs_insert(orient_hs, orients[3]);
                    TIMER_STOP(TIMER_HASHSET);

                    total_gen++;
                    if (!cl_add(&next_list, canonical)) goto oom;

                    TIMER_START(TIMER_HOLE);
                    bool hole = poly_has_hole(canonical, can_w, can_h);
                    TIMER_STOP(TIMER_HOLE);

                    if (hole) hole_all++;
                    int md = (can_w > can_h) ? can_w : can_h;
                    for (int n = md; n <= max_n; n++) {
                        results[n - 1].total++;
                        if (hole) results[n - 1].has_hole++;
                        else      results[n - 1].no_hole++;
                    }
                }
            }
        }

        fprintf(stderr, "  [枚举] 格=%2d 本代=%d 生成=%d 累计=%d "
                "洞=%d 快跳=%d\n",
                size, cur_list.total, next_list.total, total_gen,
                hole_all, fast_skip);

        if (next_list.total == 0) break;
        cl_free(&cur_list);
        cl_swap(&cur_list, &next_list);
        cl_init(&next_list);
    }

    cl_free(&cur_list); cl_free(&next_list);
    hs_free(orient_hs); hs_free(canon_hs);
    return results;

oom:
    cl_free(&cur_list); cl_free(&next_list);
    free(results);
    hs_free(orient_hs); hs_free(canon_hs);
    return NULL;
}
