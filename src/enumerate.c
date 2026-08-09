/**
 * enumerate.c — 多联骨牌枚举引擎
 *
 * 基于 Redelmeier 算法的 one-sided polyomino 枚举：
 *   1. 从单格开始，逐格扩展（生长法）
 *   2. 每次扩展后计算 one-sided canonical form（4 旋转，取最小）
 *   3. 开放寻址哈希集合去重
 *   4. Flood fill 检测亏格（有洞/无洞分类）
 *   5. 按包围矩形 ≤ n×n 分别累积计数
 *
 * 参考文献：
 *   Redelmeier, D. H. (1981). Counting polyominoes: yet another attack.
 *   Discrete Mathematics, 36(2), 191-203.
 */

#include "enumerate.h"
#include "hashset.h"

/* ================================================================
 * 内部辅助：位图 ← → 坐标列表
 * ================================================================ */

/**
 * 从规范化掩码中提取格子坐标列表
 * 掩码已左上角对齐 (0,0)，stride=STRIDE
 */
static void mask_to_cells(mask_t mask, int w, int h,
                          int cells_r[], int cells_c[], int *count) {
    *count = 0;
    for (int r = 0; r < h; r++) {
        for (int c = 0; c < w; c++) {
            if (mask & (1ULL << (r * STRIDE + c))) {
                cells_r[*count] = r;
                cells_c[*count] = c;
                (*count)++;
            }
        }
    }
}

/**
 * 从掩码中提取包围矩形尺寸（w, h）
 * 注意：掩码可能有内部空行/列，但规范化保证左上角对齐
 */
static void mask_get_extent(mask_t mask, int *w, int *h) {
    *w = 0; *h = 0;
    for (int r = 0; r < MAX_N; r++) {
        for (int c = 0; c < MAX_N; c++) {
            if (mask & (1ULL << (r * STRIDE + c))) {
                if (c + 1 > *w) *w = c + 1;
                if (r + 1 > *h) *h = r + 1;
            }
        }
    }
}

/* ================================================================
 * 规范化 — 平移使最小行/列为 0
 * ================================================================ */

static mask_t normalize_mask(mask_t cells, int w, int h,
                              int *out_w, int *out_h) {
    int min_r = h, max_r = -1, min_c = w, max_c = -1;
    for (int r = 0; r < h; r++) {
        for (int c = 0; c < w; c++) {
            if (cells & (1ULL << (r * STRIDE + c))) {
                if (r < min_r) min_r = r;
                if (r > max_r) max_r = r;
                if (c < min_c) min_c = c;
                if (c > max_c) max_c = c;
            }
        }
    }

    if (min_r > max_r) {
        *out_w = 0; *out_h = 0;
        return 0;
    }

    mask_t result = 0;
    for (int r = min_r; r <= max_r; r++) {
        for (int c = min_c; c <= max_c; c++) {
            if (cells & (1ULL << (r * STRIDE + c))) {
                result |= 1ULL << ((r - min_r) * STRIDE + (c - min_c));
            }
        }
    }

    *out_w = max_c - min_c + 1;
    *out_h = max_r - min_r + 1;
    return result;
}

/* ================================================================
 * 旋转 — 90° 顺时针
 * ================================================================ */

static mask_t rotate90(mask_t cells, int w, int h, int *out_w, int *out_h) {
    mask_t result = 0;
    for (int r = 0; r < h; r++) {
        for (int c = 0; c < w; c++) {
            if (cells & (1ULL << (r * STRIDE + c))) {
                int nr = c;                /* 新行 = 旧列 */
                int nc = h - 1 - r;        /* 新列 = 旧高 - 1 - 旧行 */
                result |= 1ULL << (nr * STRIDE + nc);
            }
        }
    }
    *out_w = h;
    *out_h = w;
    return result;
}

/* ================================================================
 * One-sided 规范化形式
 *
 * 对 4 个旋转（0°, 90°, 180°, 270°）分别平移规范化，取位图值最小者。
 * 这等价于考虑旋转+平移但不考虑翻转（one-sided）。
 * ================================================================ */

static mask_t poly_canonical_one_sided(mask_t cells, int w, int h,
                                        int *out_w, int *out_h) {
    mask_t best = UINT64_MAX;
    int best_w = 0, best_h = 0;

    mask_t cur = cells;
    int cw = w, ch = h;

    for (int rot = 0; rot < 4; rot++) {
        int nw, nh;
        mask_t norm = normalize_mask(cur, cw, ch, &nw, &nh);

        if (norm < best) {
            best = norm;
            best_w = nw;
            best_h = nh;
        }

        /* 旋转供下一轮使用 */
        cur = rotate90(cur, cw, ch, &cw, &ch);
    }

    *out_w = best_w;
    *out_h = best_h;
    return best;
}

/* ================================================================
 * 洞检测 — Flood Fill
 *
 * 在包围矩形外扩一圈（共 (h+2)×(w+2) 格），
 * 从 (0,0) 做 BFS 遍历所有可达的非房间格子。
 * 若包围矩形内部存在未被访问的空格 → 该格被房间完全包围 → 有洞（亏格 ≥ 1）。
 * ================================================================ */

static bool poly_has_hole(mask_t cells, int w, int h) {
    int gh = h + 2;  /* 外扩一圈的高度 */
    int gw = w + 2;

    /* 小网格：MAX_N ≤ 6，gh, gw ≤ 10 */
    bool occupied[GRID_PAD][GRID_PAD];
    bool visited[GRID_PAD][GRID_PAD];
    memset(occupied, 0, sizeof(occupied));
    memset(visited, 0, sizeof(visited));

    /* 标记房间格子（偏移 +1） */
    for (int r = 0; r < h; r++) {
        for (int c = 0; c < w; c++) {
            if (cells & (1ULL << (r * STRIDE + c))) {
                occupied[r + 1][c + 1] = true;
            }
        }
    }

    /* BFS 队列 */
    int qr[GRID_PAD * GRID_PAD], qc[GRID_PAD * GRID_PAD];
    int head = 0, tail = 0;
    qr[tail] = 0; qc[tail] = 0; tail++;
    visited[0][0] = true;

    static const int dr[] = {-1, 1, 0, 0};
    static const int dc[] = {0, 0, -1, 1};

    while (head < tail) {
        int r = qr[head], c = qc[head];
        head++;

        for (int d = 0; d < 4; d++) {
            int nr = r + dr[d];
            int nc = c + dc[d];
            if (nr >= 0 && nr < gh && nc >= 0 && nc < gw) {
                if (!visited[nr][nc] && !occupied[nr][nc]) {
                    visited[nr][nc] = true;
                    qr[tail] = nr; qc[tail] = nc;
                    tail++;
                }
            }
        }
    }

    /* 检查包围矩形内部未被访问的空格 */
    for (int r = 1; r <= h; r++) {
        for (int c = 1; c <= w; c++) {
            if (!occupied[r][c] && !visited[r][c]) {
                return true;  /* 发现洞 */
            }
        }
    }

    return false;
}

/* ================================================================
 * 动态数组辅助宏
 * ================================================================ */

#define ENSURE_CAP(arr, cap_var, needed)                                \
    do {                                                                \
        if ((cap_var) < (needed)) {                                     \
            int new_cap = (cap_var) ? (cap_var) * 2 : 1024;            \
            while (new_cap < (needed)) new_cap *= 2;                   \
            mask_t *tmp = (mask_t *)realloc((arr),                     \
                                   (size_t)new_cap * sizeof(mask_t));  \
            if (!tmp) goto oom;                                         \
            (arr) = tmp;                                                \
            (cap_var) = new_cap;                                        \
        }                                                               \
    } while (0)

/* ================================================================
 * 主枚举函数
 * ================================================================ */

RoomCount *enumerate_all(int max_n, int *out_count) {
    *out_count = max_n;
    RoomCount *results = (RoomCount *)calloc((size_t)max_n, sizeof(RoomCount));
    if (!results) return NULL;

    for (int i = 0; i < max_n; i++) {
        results[i].n = i + 1;
    }

    /* ---------- 哈希集合（去重） ---------- */
    HashSet *hs = hs_create(1 << 20);  /* 初始 1M 容量 */
    if (!hs) { free(results); return NULL; }

    /* ---------- 当前代 / 下一代掩码数组 ---------- */
    int cur_cap = 0, next_cap = 0;
    mask_t *cur_masks = NULL;
    mask_t *next_masks = NULL;
    int cur_count = 0, next_count = 0;

    /* ---------- 细胞坐标工作数组 ---------- */
    int cells_r[MAX_CELLS], cells_c[MAX_CELLS];

    /* ---------- 起始：单格房间 ---------- */
    mask_t start_mask = 1ULL;  /* 格子 (0,0) */
    hs_insert(hs, start_mask);

    ENSURE_CAP(cur_masks, cur_cap, 1);
    cur_masks[0] = start_mask;
    cur_count = 1;

    /* 单格房间对所有 n ≥ 1 都有效 */
    for (int n = 1; n <= max_n; n++) {
        results[n - 1].total++;
        results[n - 1].no_hole++;
    }

    /* ============================================================
     * 生长循环：从 size = 1 逐步扩展到 max_n²
     * ============================================================ */
    int total_generated = 1;   /* 含起始单格 */
    int hole_count_all = 0;

    for (int size = 1; size < max_n * max_n; size++) {
        next_count = 0;

        for (int pi = 0; pi < cur_count; pi++) {
            mask_t pmask = cur_masks[pi];

            /* 提取包围矩形尺寸 */
            int pw, ph;
            mask_get_extent(pmask, &pw, &ph);

            /* 提取格子坐标 */
            int cell_count;
            mask_to_cells(pmask, pw, ph, cells_r, cells_c, &cell_count);

            /* ---------- 计算前沿（可扩展的空邻居） ---------- */
            /* 用小型 visited 网格标记前沿，去重 */
            bool occ[GRID_PAD][GRID_PAD];
            bool in_front[GRID_PAD][GRID_PAD];
            memset(occ, 0, sizeof(occ));
            memset(in_front, 0, sizeof(in_front));

            for (int i = 0; i < cell_count; i++) {
                occ[cells_r[i] + 1][cells_c[i] + 1] = true;
            }

            int fr[256], fc[256];  /* 前沿最多约 4×(size+2) ≤ 152 */
            int fcount = 0;

            static const int dr[] = {-1, 1, 0, 0};
            static const int dc[] = {0, 0, -1, 1};

            for (int i = 0; i < cell_count; i++) {
                int r = cells_r[i], c = cells_c[i];
                for (int d = 0; d < 4; d++) {
                    int nr = r + dr[d];
                    int nc = c + dc[d];

                    if (occ[nr + 1][nc + 1]) continue;     /* 已占用 */
                    if (in_front[nr + 1][nc + 1]) continue; /* 已在前沿 */

                    /* 检查扩展后包围矩形是否超出 max_n */
                    int new_w = pw, new_h = ph;
                    if (nc < 0) new_w++;
                    else if (nc >= pw) new_w = nc + 1;
                    if (nr < 0) new_h++;
                    else if (nr >= ph) new_h = nr + 1;

                    if (new_w > max_n || new_h > max_n) continue;

                    in_front[nr + 1][nc + 1] = true;
                    fr[fcount] = nr;
                    fc[fcount] = nc;
                    fcount++;
                }
            }

            /* ---------- 逐个前沿格子扩展 ---------- */
            for (int fi = 0; fi < fcount; fi++) {
                int nr = fr[fi], nc = fc[fi];

                /* 计算平移量 */
                int shift_r = (nr < 0) ? 1 : 0;
                int shift_c = (nc < 0) ? 1 : 0;

                /* 构造新掩码（逐行移位，避免跨行泄漏） */
                mask_t new_mask = 0;
                for (int r = 0; r < ph; r++) {
                    mask_t row = (pmask >> (r * STRIDE)) & ((1ULL << pw) - 1);
                    new_mask |= (row << shift_c) << ((r + shift_r) * STRIDE);
                }
                /* 添加新格子 */
                new_mask |= 1ULL << ((nr + shift_r) * STRIDE + (nc + shift_c));

                /* 计算新包围矩形（规范化前） */
                int raw_w = pw;
                if (nc < 0) raw_w++;
                else if (nc >= pw) raw_w = nc + 1;
                int raw_h = ph;
                if (nr < 0) raw_h++;
                else if (nr >= ph) raw_h = nr + 1;

                /* 计算 one-sided 规范化形式 */
                int can_w, can_h;
                mask_t canonical = poly_canonical_one_sided(
                    new_mask, raw_w, raw_h, &can_w, &can_h);

                /* 去重检查 */
                if (!hs_insert(hs, canonical)) continue;

                total_generated++;

                /* 添加至下一代列表 */
                ENSURE_CAP(next_masks, next_cap, next_count + 1);
                next_masks[next_count++] = canonical;

                /* ---------- 分类统计 ---------- */
                bool hole = poly_has_hole(canonical, can_w, can_h);
                if (hole) hole_count_all++;

                int max_dim = (can_w > can_h) ? can_w : can_h;
                for (int n = max_dim; n <= max_n; n++) {
                    results[n - 1].total++;
                    if (hole) {
                        results[n - 1].has_hole++;
                    } else {
                        results[n - 1].no_hole++;
                    }
                }
            }
        }

        /* 打印进度 */
        fprintf(stderr,
                "  [枚举] 格子数=%2d  本代形状数=%d  生成%d格形状=%d  "
                "累计唯一形状=%d  有洞=%d\n",
                size, cur_count, size + 1, next_count,
                total_generated, hole_count_all);

        if (next_count == 0) break;

        /* 交换当前代 / 下一代 */
        {
            mask_t *tmp_masks = cur_masks;
            int tmp_cap = cur_cap;
            cur_masks = next_masks;
            cur_cap = next_cap;
            cur_count = next_count;
            next_masks = tmp_masks;
            next_cap = tmp_cap;
            next_count = 0;
        }
    }

    /* ---------- 清理 ---------- */
    free(cur_masks);
    free(next_masks);
    hs_free(hs);

    return results;

oom:
    free(results);
    free(cur_masks);
    free(next_masks);
    hs_free(hs);
    return NULL;
}
