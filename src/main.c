/**
 * main.c — 入口程序
 *
 * 运行多联骨牌房间枚举，输出 n=1..max_n 的结果表。
 * 用法: room-count.exe [n]
 *   不指定 n 时默认运行到 MAX_N=6
 */

#include "enumerate.h"
#include <time.h>

int main(int argc, char **argv) {
    int max_n = MAX_N;
    if (argc >= 2) {
        max_n = atoi(argv[1]);
        if (max_n < 1 || max_n > MAX_N) {
            fprintf(stderr, "错误：n 必须在 1..%d 之间\n", MAX_N);
            return 1;
        }
    }

    printf("═══════════════════════════════════════════════════════════\n");
    printf("  多联骨牌房间计数 — One-sided Polyomino 枚举\n");
    printf("  n×n 正方形网格，每个格子是一个房间单元\n");
    printf("  房间 = 墙+门围成的连通区域（≤ n×n）\n");
    printf("  去重规则：允许旋转+平移，禁止翻转（One-sided）\n");
    printf("  目标 n = %d\n", max_n);
    printf("═══════════════════════════════════════════════════════════\n\n");

    clock_t start = clock();

    int count;
    RoomCount *results = enumerate_all(max_n, &count);

    clock_t elapsed = clock() - start;
    double seconds = (double)elapsed / CLOCKS_PER_SEC;

    if (!results) {
        fprintf(stderr, "错误：枚举失败（内存不足？）\n");
        return 1;
    }

    printf("\n");
    printf("  ╔══════════════════════════════════════════════════════╗\n");
    printf("  ║              枚举结果（One-sided）                    ║\n");
    printf("  ╠═════╤══════════╤══════════╤══════════╤═══════════════╣\n");
    printf("  ║  n  │  总房间数  │  无洞(亏格0) │  有洞(亏格≥1) ║\n");
    printf("  ╟─────┼──────────┼──────────┼──────────┼───────────────╢\n");

    for (int i = 0; i < count; i++) {
        printf("  ║ %-3d │ %8d │ %8d │ %8d ║\n",
               results[i].n, results[i].total,
               results[i].no_hole, results[i].has_hole);
    }

    printf("  ╚═════╧══════════╧══════════╧══════════╧═══════════════╝\n");

    printf("\n  ⏱ 运行耗时: %.2f 秒\n", seconds);
    printf("  📊 唯一形状总数: %d (≤ %d×%d 包围矩形)\n",
           results[count - 1].total, max_n, max_n);

    /* 输出便于复制的结果摘要 */
    printf("\n  ── 结果摘要（n, total, no_hole, has_hole）──\n");
    for (int i = 0; i < count; i++) {
        printf("  n=%d: total=%d, no_hole=%d, has_hole=%d\n",
               results[i].n, results[i].total,
               results[i].no_hole, results[i].has_hole);
    }

    free(results);
    return 0;
}
