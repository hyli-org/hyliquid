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
    <div class="min-h-0 grow flex flex-col rounded-md border border-neutral-800">
        <!-- Table -->
        <div class="flex-1 overflow-auto">
            <table class="w-full text-sm">
                <thead class="sticky top-0 bg-neutral-900/60 text-neutral-400">
                    <tr>
                        <th class="px-3 py-2 text-left font-medium cursor-pointer hover:text-neutral-200 select-none"
                            @click="handleColumnSort('created_at')"
                            :title="`Sort by Created At ${getSortIcon('created_at')}`">
                            Created At {{ getSortIcon("created_at") }}
                        </th>
                        <th class="px-3 py-2 text-left font-medium">Symbol</th>
                        <th class="px-3 py-2 text-left font-medium">Side</th>
                        <th class="px-3 py-2 text-left font-medium cursor-pointer hover:text-neutral-200 select-none"
                            @click="handleColumnSort('qty')" :title="`Sort by Quantity ${getSortIcon('qty')}`">
                            Qty {{ getSortIcon("qty") }}
                        </th>
                        <th class="px-3 py-2 text-left font-medium">Qty Remaining</th>
                        <th class="px-3 py-2 text-left font-medium cursor-pointer hover:text-neutral-200 select-none"
                            @click="handleColumnSort('price')" :title="`Sort by Price ${getSortIcon('price')}`">
                            Price {{ getSortIcon("price") }}
                        </th>
                        <th class="px-3 py-2 text-left font-medium cursor-pointer hover:text-neutral-200 select-none"
                            @click="handleColumnSort('status')" :title="`Sort by Status ${getSortIcon('status')}`">
                            Status {{ getSortIcon("status") }}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    <tr v-if="loading">
                        <td colspan="6" class="px-3 py-3 text-neutral-400">Loading orders…</td>
                    </tr>
                    <tr v-else-if="error">
                        <td colspan="6" class="px-3 py-3 text-rose-400">{{ error }}</td>
                    </tr>
                    <tr v-else-if="orders.length === 0">
                        <td colspan="6" class="px-3 py-3 text-neutral-400">No orders found</td>
                    </tr>
                    <tr v-for="o in orders" :key="o.symbol + o.price + o.type" class="border-t border-neutral-900">
                        <td class="px-3 py-2">{{ o.created_at.toLocaleString() }}</td>
                        <td class="px-3 py-2">{{ o.symbol }}</td>
                        <td class="px-3 py-2" :class="o.side === 'bid' ? 'text-emerald-400' : 'text-rose-400'">
                            {{ o.side }}
                        </td>
                        <td class="px-3 py-2 tabular-nums">{{ instrumentsState.toRealQty(o.symbol, o.qty) }}</td>
                        <td class="px-3 py-2 tabular-nums">
                            {{ instrumentsState.toRealQty(o.symbol, o.qty_remaining) }}
                        </td>
                        <td class="px-3 py-2 tabular-nums">
                            {{ o.type }} @
                            {{ o.price ? instrumentsState.toRealPrice(o.symbol, o.price).toLocaleString() : "market" }}
                        </td>
                        <td class="px-3 py-2">{{ o.status }}</td>
                    </tr>
                </tbody>
            </table>
        </div>

        <!-- Pagination controls -->
        <div v-if="pagination"
            class="flex items-center justify-between p-3 border-t border-neutral-800 bg-neutral-900/30">
            <div class="flex items-center gap-4">
                <!-- Page size selector -->
                <div class="flex items-center gap-2">
                    <label class="text-sm text-neutral-400">Per page:</label>
                    <select :value="activityState.ordersPageSize" @change="handlePageSizeChange"
                        class="px-2 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200">
                        <option v-for="size in pageSizeOptions" :key="size" :value="size">
                            {{ size }}
                        </option>
                    </select>
                </div>

                <!-- Pagination info -->
                <div class="text-sm text-neutral-400">
                    Showing {{ (pagination.page - 1) * pagination.limit + 1 }}-{{
                        Math.min(pagination.page * pagination.limit, pagination.total)
                    }}
                    of {{ pagination.total }}
                </div>
            </div>

            <!-- Pagination navigation -->
            <div v-if="pagination.total_pages > 1" class="flex items-center gap-2">
                <button @click="prevOrdersPage" :disabled="!pagination.has_prev"
                    class="px-3 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed">
                    Previous
                </button>

                <span class="text-sm text-neutral-400">
                    Page {{ pagination.page }} of {{ pagination.total_pages }}
                </span>

                <button @click="nextOrdersPage" :disabled="!pagination.has_next"
                    class="px-3 py-1 text-sm bg-neutral-800 border border-neutral-700 rounded text-neutral-200 hover:bg-neutral-700 disabled:opacity-50 disabled:cursor-not-allowed">
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
