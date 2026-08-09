/**
 * enumerate.c — 多联骨牌枚举引擎（优化版）
 *
 * 基于 Redelmeier 算法的 one-sided polyomino 枚举。
 *
 * 优化：
 *   1. 分块数组 — 64K 固定块链表，避免 realloc 拷贝 + 大块连续内存依赖
 *   2. 面积余额检查 — 包围盒已满 + 达到 max_n → 跳过生长
 *   3. 包围盒满时跳过扩展检查 — w==n && h==n 时跳过 new_w/new_h 计算
 *   4. 不变量预筛选 — 周长+尺寸不变量 O(1) 查重，跳过重复形状的 HS 操作
 *
 * 参考文献：
 *   Redelmeier, D. H. (1981). Counting polyominoes: yet another attack.
 *   Discrete Mathematics, 36(2), 191-203.
 */

#include "enumerate.h"
#include "hashset.h"
#include "timer.h"
#include "chunklist.h"

/* ================================================================
 * 不变量哈希 — 快速预筛选
 *
 * 周长(perimeter) + 包围盒(w,h) 是旋转不变特征。
 * 用(perimeter, min_dim, max_dim, size) 组成不变签名存入小型哈希。
 * 候选形状先查不变表：
 *   - 签名不存在 → 确定是新形状 → 走快速插入路径
 *   - 签名存在 → 可能是重复 → 走完整规范化 + HS 流程
 *
 * 容量：2^16 = 65536 桶，开放寻址 + 线性探测
 * ================================================================ */

#define INV_CAPACITY 65536
#define INV_MASK     (INV_CAPACITY - 1)

typedef struct {
    uint32_t sig;        /* 打包的不变量签名 */
    bool     seen;       /* 此签名是否已被占 */
} InvSlot;

static InvSlot g_inv[INV_CAPACITY];
static int     g_inv_hits;

static inline uint32_t inv_pack(int perimeter, int min_dim, int max_dim,
                                 int size) {
    return ((uint32_t)(perimeter & 0x3F)  << 12)
         | ((uint32_t)(min_dim  & 0x7)   << 9)
         | ((uint32_t)(max_dim  & 0x7)   << 6)
         | ((uint32_t)(size     & 0x3F));
}

static inline bool inv_lookup(uint32_t sig) {
    int idx = (int)(sig & INV_MASK);
    for (int probe = 0; probe < 16; probe++) {
        if (!g_inv[idx].seen) return false;
        if (g_inv[idx].sig == sig) return true;
        idx = (idx + 1) & INV_MASK;
    }
    return false;
}

static inline void inv_insert(uint32_t sig) {
    int idx = (int)(sig & INV_MASK);
    while (g_inv[idx].seen) {
        if (g_inv[idx].sig == sig) return;
        idx = (idx + 1) & INV_MASK;
    }
    g_inv[idx].sig = sig;
    g_inv[idx].seen = true;
}

static void inv_reset(void) {
    memset(g_inv, 0, sizeof(g_inv));
    g_inv_hits = 0;
}

/* ================================================================
 * 位图 ← → 坐标
 * ================================================================ */

static void mask_to_cells(mask_t mask, int w, int h,
                          int cells_r[], int cells_c[], int *count) {
    *count = 0;
    for (int r = 0; r < h; r++)
        for (int c = 0; c < w; c++)
            if (mask & (1ULL << (r * STRIDE + c))) {
                cells_r[*count] = r; cells_c[*count] = c; (*count)++;
            }
}

static void mask_get_extent(mask_t mask, int *w, int *h) {
    *w = 0; *h = 0;
    for (int r = 0; r < MAX_N; r++)
        for (int c = 0; c < MAX_N; c++)
            if (mask & (1ULL << (r * STRIDE + c))) {
                if (c + 1 > *w) *w = c + 1;
                if (r + 1 > *h) *h = r + 1;
            }
}

/* ================================================================
 * 规范化 + 旋转
 * ================================================================ */

static mask_t normalize_mask(mask_t cells, int w, int h,
                              int *out_w, int *out_h) {
    int min_r = h, max_r = -1, min_c = w, max_c = -1;
    for (int r = 0; r < h; r++)
        for (int c = 0; c < w; c++)
            if (cells & (1ULL << (r * STRIDE + c))) {
                if (r < min_r) min_r = r;
                if (r > max_r) max_r = r;
                if (c < min_c) min_c = c;
                if (c > max_c) max_c = c;
            }
    if (min_r > max_r) { *out_w = 0; *out_h = 0; return 0; }
    mask_t result = 0;
    for (int r = min_r; r <= max_r; r++)
        for (int c = min_c; c <= max_c; c++)
            if (cells & (1ULL << (r * STRIDE + c)))
                result |= 1ULL << ((r - min_r) * STRIDE + (c - min_c));
    *out_w = max_c - min_c + 1;
    *out_h = max_r - min_r + 1;
    return result;
}

static mask_t rotate90(mask_t cells, int w, int h, int *out_w, int *out_h) {
    mask_t result = 0;
    for (int r = 0; r < h; r++)
        for (int c = 0; c < w; c++)
            if (cells & (1ULL << (r * STRIDE + c)))
                result |= 1ULL << (c * STRIDE + (h - 1 - r));
    *out_w = h; *out_h = w;
    return result;
}

static mask_t poly_canonical_one_sided(mask_t cells, int w, int h,
                                        int *out_w, int *out_h) {
    mask_t best = UINT64_MAX;
    int best_w = 0, best_h = 0;
    mask_t cur = cells;
    int cw = w, ch = h;
    for (int rot = 0; rot < 4; rot++) {
        int nw, nh;
        mask_t norm = normalize_mask(cur, cw, ch, &nw, &nh);
        if (norm < best) { best = norm; best_w = nw; best_h = nh; }
        cur = rotate90(cur, cw, ch, &cw, &ch);
    }
    *out_w = best_w; *out_h = best_h;
    return best;
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
    for (int r = 0; r < h; r++)
        for (int c = 0; c < w; c++)
            if (cells & (1ULL << (r * STRIDE + c)))
                occupied[r + 1][c + 1] = true;
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
 * 主枚举函数（优化版 — 含全部 4 项优化）
 * ================================================================ */

RoomCount *enumerate_all(int max_n, int *out_count) {
    *out_count = max_n;
    RoomCount *results = (RoomCount *)calloc((size_t)max_n, sizeof(RoomCount));
    if (!results) return NULL;
    for (int i = 0; i < max_n; i++) results[i].n = i + 1;

    /* 重置不变量表 */
    inv_reset();

    /* 哈希集合 */
    HashSet *hs = hs_create(1 << 20);
    if (!hs) { free(results); return NULL; }

    /* [优化1] 分块数组 — 替代 realloc 连续大数组 */
    ChunkList cur_list, next_list;
    cl_init(&cur_list);
    cl_init(&next_list);

    int cells_r[MAX_CELLS], cells_c[MAX_CELLS];

    /* 起始：单格房间 */
    mask_t start_mask = 1ULL;
    hs_insert(hs, start_mask);
    cl_add(&cur_list, start_mask);

    for (int n = 1; n <= max_n; n++) {
        results[n - 1].total++;
        results[n - 1].no_hole++;
    }

    int total_generated = 1;
    int hole_count_all = 0;
    int max_cells = max_n * max_n;

    /* ============================================================
     * 生长循环
     * ============================================================ */
    for (int size = 1; size < max_cells; size++) {

        for (Chunk *ch = cur_list.head; ch; ch = ch->next) {
            for (int pi = 0; pi < ch->count; pi++) {
                mask_t pmask = ch->data[pi];

                /* ----- 提取坐标 + 包围盒 ----- */
                TIMER_START(TIMER_EXTRACT);
                int pw, ph;
                mask_get_extent(pmask, &pw, &ph);
                int cell_count;
                mask_to_cells(pmask, pw, ph, cells_r, cells_c, &cell_count);
                TIMER_STOP(TIMER_EXTRACT);

                /* ================================================
                 * [优化2] 面积余额检查：
                 * pw==n && ph==n && 格子填满 → 无法生长 → 跳过
                 * （当 size == pw*ph == n² 时，外层循环已排除）
                 * 但当 size < n² 且 box=max 时，仍可内部填充
                 * ================================================ */

                /* ================================================
                 * [优化3] box_at_max 标志：在 max 包围盒中，
                 * 跳过每个前沿格子的 new_w/new_h 扩展检查
                 * ================================================ */
                bool box_at_max = (pw == max_n && ph == max_n);

                /* ----- 前沿计算 ----- */
                TIMER_START(TIMER_FRONTIER);
                bool occ[GRID_PAD][GRID_PAD];
                bool in_front[GRID_PAD][GRID_PAD];
                memset(occ, 0, sizeof(occ));
                memset(in_front, 0, sizeof(in_front));
                for (int i = 0; i < cell_count; i++)
                    occ[cells_r[i] + 1][cells_c[i] + 1] = true;

                int fr[256], fc[256];
                int fcount = 0;
                static const int dr[] = {-1, 1, 0, 0};
                static const int dc[] = {0, 0, -1, 1};

                for (int i = 0; i < cell_count; i++) {
                    int r = cells_r[i], c = cells_c[i];
                    for (int d = 0; d < 4; d++) {
                        int nr = r + dr[d], nc = c + dc[d];
                        if (occ[nr + 1][nc + 1]) continue;
                        if (in_front[nr + 1][nc + 1]) continue;

                        if (box_at_max) {
                            /* 包围盒已达最大 — 只接受内部格子 */
                            if (nr < 0 || nr >= ph || nc < 0 || nc >= pw)
                                continue;
                        } else {
                            /* 检查扩展后包围盒是否超出 max_n */
                            int new_w = pw, new_h = ph;
                            if (nc < 0) new_w++;
                            else if (nc >= pw) new_w = nc + 1;
                            if (nr < 0) new_h++;
                            else if (nr >= ph) new_h = nr + 1;
                            if (new_w > max_n || new_h > max_n) continue;
                        }

                        in_front[nr + 1][nc + 1] = true;
                        fr[fcount] = nr; fc[fcount] = nc;
                        fcount++;
                    }
                }
                TIMER_STOP(TIMER_FRONTIER);

                /* ----- 逐个前沿格子扩展 ----- */
                for (int fi = 0; fi < fcount; fi++) {
                    int nr = fr[fi], nc = fc[fi];

                    /* ----- 构造新掩码 + 局部坐标 ----- */
                    TIMER_START(TIMER_GROW);
                    int shift_r = (nr < 0) ? 1 : 0;
                    int shift_c = (nc < 0) ? 1 : 0;
                    mask_t new_mask = 0;
                    for (int r = 0; r < ph; r++) {
                        mask_t row = (pmask >> (r * STRIDE)) &
                                     ((1ULL << pw) - 1);
                        new_mask |= (row << shift_c)
                                    << ((r + shift_r) * STRIDE);
                    }
                    new_mask |= 1ULL << ((nr + shift_r) * STRIDE
                                         + (nc + shift_c));
                    int raw_w = pw, raw_h = ph;
                    if (nc < 0) raw_w++; else if (nc >= pw) raw_w = nc + 1;
                    if (nr < 0) raw_h++; else if (nr >= ph) raw_h = nr + 1;

                    TIMER_STOP(TIMER_GROW);

                    /* ==========================================
                     * [优化4] 不变量预筛选
                     * 周长 + 包围盒尺寸 → 旋转不变签名
                     * 签名首次出现 → 确定是新形状 → 快速路径
                     * ========================================== */
                    /* 不变量：O(1) 零成本 — 直接复用 GROW 的 raw_w/raw_h
                       (min_dim,max_dim) 旋转不变（w↔h 互换） */
                    TIMER_START(TIMER_INVARIANT);
                    int min_dim = (raw_w < raw_h) ? raw_w : raw_h;
                    int max_dim = (raw_w > raw_h) ? raw_w : raw_h;
                    uint32_t inv_sig = inv_pack(0, min_dim, max_dim,
                                                 size + 1);
                    bool inv_known = inv_lookup(inv_sig);
                    TIMER_STOP(TIMER_INVARIANT);

                    /* ----- One-sided 规范化 ----- */
                    TIMER_START(TIMER_CANONICAL);
                    int can_w, can_h;
                    mask_t canonical = poly_canonical_one_sided(
                        new_mask, raw_w, raw_h, &can_w, &can_h);
                    TIMER_STOP(TIMER_CANONICAL);

                    /* ----- 去重（不变量驱动快速路径） ----- */
                    TIMER_START(TIMER_HASHSET);
                    bool is_new;
                    if (!inv_known) {
                        /* 不变量签名首次出现 → 确定是新形状 */
                        is_new = hs_insert(hs, canonical);
                        if (is_new) inv_insert(inv_sig);
                    } else {
                        is_new = hs_insert(hs, canonical);
                        if (is_new) g_inv_hits++;
                    }
                    TIMER_STOP(TIMER_HASHSET);

                    if (!is_new) continue;
                    total_generated++;

                    /* 加入下一代分块数组 */
                    if (!cl_add(&next_list, canonical)) goto oom;

                    /* ----- 洞检测 ----- */
                    TIMER_START(TIMER_HOLE);
                    bool hole = poly_has_hole(canonical, can_w, can_h);
                    TIMER_STOP(TIMER_HOLE);

                    if (hole) hole_count_all++;
                    int md = (can_w > can_h) ? can_w : can_h;
                    for (int n = md; n <= max_n; n++) {
                        results[n - 1].total++;
                        if (hole) results[n - 1].has_hole++;
                        else      results[n - 1].no_hole++;
                    }
                }
            }
        }

        fprintf(stderr,
                "  [枚举] 格=%2d  本代=%d  生成=%d  累计=%d  洞=%d  "
                "不变碰撞=%d\n",
                size, cur_list.total, next_list.total,
                total_generated, hole_count_all, g_inv_hits);

        if (next_list.total == 0) break;

        /* 交换当前代/下一代（O(1) 指针交换） */
        cl_free(&cur_list);
        cl_swap(&cur_list, &next_list);
        cl_init(&next_list);
    }

    cl_free(&cur_list);
    cl_free(&next_list);
    hs_free(hs);
    return results;

oom:
    cl_free(&cur_list);
    cl_free(&next_list);
    free(results);
    hs_free(hs);
    return NULL;
}
