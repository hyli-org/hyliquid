<script setup lang="ts">
import type { Order, PaginationInfo } from "../trade";
import { instrumentsState, activityState, nextOrdersPage, prevOrdersPage, changeOrdersSorting } from "../trade";

defineProps<{
    orders: Order[];
    loading?: boolean;
    error?: string | null;
    pagination?: PaginationInfo | null;
}>();

const pageSizeOptions = [10, 20, 50, 100];

const handleColumnSort = async (column: string) => {
    // If clicking the same column, toggle sort order
    if (activityState.ordersSortBy === column) {
        const newOrder = activityState.ordersSortOrder === "asc" ? "desc" : "asc";
        await changeOrdersSorting(column, newOrder);
    } else {
        // If clicking a different column, set it as the new sort column with default desc order
        await changeOrdersSorting(column, "desc");
    }
};

const getSortIcon = (column: string) => {
    if (activityState.ordersSortBy !== column) return "";
    return activityState.ordersSortOrder === "asc" ? "↑" : "↓";
};

const handlePageSizeChange = async (event: Event) => {
    const target = event.target as HTMLSelectElement;
    const newPageSize = parseInt(target.value);
    activityState.ordersPageSize = newPageSize;
    await changeOrdersSorting(activityState.ordersSortBy, activityState.ordersSortOrder);
    // Reset to page 1 when changing page size
    activityState.ordersCurrentPage = 1;
};
</script>

<template>
    <div class="min-h-0 grow flex flex-col">
        <!-- Table -->
        <div class="flex-1 overflow-auto">
            <table class="w-full text-sm">
                <thead class="sticky top-0 bg-[var(--surface-header)] text-[var(--text-muted)]">
                    <tr>
                        <th
                            class="select-none px-3 py-2 text-left font-medium hover:text-[var(--text-primary)] cursor-pointer"
                            @click="handleColumnSort('created_at')"
                            :title="`Sort by Created At ${getSortIcon('created_at')}`"
                        >
                            Created At {{ getSortIcon("created_at") }}
                        </th>
                        <th class="px-3 py-2 text-left font-medium">Symbol</th>
                        <th class="px-3 py-2 text-left font-medium">Side</th>
                        <th
                            class="select-none px-3 py-2 text-left font-medium hover:text-[var(--text-primary)] cursor-pointer"
                            @click="handleColumnSort('qty')"
                            :title="`Sort by Quantity ${getSortIcon('qty')}`"
                        >
                            Qty {{ getSortIcon("qty") }}
                        </th>
                        <th class="px-3 py-2 text-left font-medium">Qty Remaining</th>
                        <th
                            class="select-none px-3 py-2 text-left font-medium hover:text-[var(--text-primary)] cursor-pointer"
                            @click="handleColumnSort('price')"
                            :title="`Sort by Price ${getSortIcon('price')}`"
                        >
                            Price {{ getSortIcon("price") }}
                        </th>
                        <th
                            class="select-none px-3 py-2 text-left font-medium hover:text-[var(--text-primary)] cursor-pointer"
                            @click="handleColumnSort('status')"
                            :title="`Sort by Status ${getSortIcon('status')}`"
                        >
                            Status {{ getSortIcon("status") }}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    <tr v-if="loading">
                        <td colspan="6" class="px-3 py-3 text-[var(--text-muted)]">Loading orders…</td>
                    </tr>
                    <tr v-else-if="error">
                        <td colspan="6" class="px-3 py-3 text-[var(--sell-color)]">{{ error }}</td>
                    </tr>
                    <tr v-else-if="orders.length === 0">
                        <td colspan="6" class="px-3 py-3 text-[var(--text-muted)]">No orders found</td>
                    </tr>
                    <tr
                        v-for="o in orders"
                        :key="o.symbol + o.price + o.type"
                        class="border-t border-[var(--table-row-border)]"
                    >
                        <td class="px-3 py-2 text-[var(--text-secondary)]">{{ o.created_at.toLocaleString() }}</td>
                        <td class="px-3 py-2 text-[var(--text-primary)]">{{ o.symbol }}</td>
                        <td
                            class="px-3 py-2"
                            :class="o.side === 'bid' ? 'text-[var(--buy-color)]' : 'text-[var(--sell-color)]'"
                        >
                            {{ o.side }}
                        </td>
                        <td class="px-3 py-2 tabular-nums text-[var(--text-secondary)]">
                            {{ instrumentsState.toRealQty(o.symbol, o.qty) }}
                        </td>
                        <td class="px-3 py-2 tabular-nums text-[var(--text-secondary)]">
                            {{ instrumentsState.toRealQty(o.symbol, o.qty_remaining) }}
                        </td>
                        <td class="px-3 py-2 tabular-nums text-[var(--text-secondary)]">
                            {{ o.type }} @
                            {{ o.price ? instrumentsState.toRealPrice(o.symbol, o.price).toLocaleString() : "market" }}
                        </td>
                        <td class="px-3 py-2 text-[var(--text-secondary)]">{{ o.status }}</td>
                    </tr>
                </tbody>
            </table>
        </div>

        <!-- Pagination controls -->
        <div
            v-if="pagination"
            class="flex items-center justify-between border-t border-[var(--border-default)] bg-[var(--surface-header)] p-3"
        >
            <div class="flex items-center gap-4">
                <!-- Page size selector -->
                <div class="flex items-center gap-2">
                    <label class="text-sm text-[var(--text-muted)]">Per page:</label>
                    <select
                        :value="activityState.ordersPageSize"
                        @change="handlePageSizeChange"
                        class="rounded border border-[var(--border-default)] bg-[var(--surface-input)] px-2 py-1 text-sm text-[var(--text-secondary)] focus:border-[var(--accent)] focus:outline-none"
                    >
                        <option v-for="size in pageSizeOptions" :key="size" :value="size">
                            {{ size }}
                        </option>
                    </select>
                </div>

                <!-- Pagination info -->
                <div class="text-sm text-[var(--text-muted)]">
                    Showing {{ (pagination.page - 1) * pagination.limit + 1 }}-{{
                        Math.min(pagination.page * pagination.limit, pagination.total)
                    }}
                    of {{ pagination.total }}
                </div>
            </div>

            <!-- Pagination navigation -->
            <div v-if="pagination.total_pages > 1" class="flex items-center gap-2">
                <button
                    @click="prevOrdersPage"
                    :disabled="!pagination.has_prev"
                    class="rounded border border-[var(--border-default)] bg-[var(--surface-input)] px-3 py-1 text-sm text-[var(--text-secondary)] transition hover:border-[var(--border-accent)] hover:text-[var(--text-accent)] disabled:cursor-not-allowed disabled:opacity-50"
                >
                    Previous
                </button>

                <span class="text-sm text-[var(--text-muted)]">
                    Page {{ pagination.page }} of {{ pagination.total_pages }}
                </span>

                <button
                    @click="nextOrdersPage"
                    :disabled="!pagination.has_next"
                    class="rounded border border-[var(--border-default)] bg-[var(--surface-input)] px-3 py-1 text-sm text-[var(--text-secondary)] transition hover:border-[var(--border-accent)] hover:text-[var(--text-accent)] disabled:cursor-not-allowed disabled:opacity-50"
                >
                    Next
                </button>
            </div>
        </div>
    </div>
</template>

<style scoped>
.tabular-nums {
    font-variant-numeric: tabular-nums;
}
</style>
