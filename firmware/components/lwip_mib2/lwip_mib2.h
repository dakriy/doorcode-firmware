/*
 * Part of the Rust bindings: they fail to generate unless they see lwIP's
 * statistics with the MIB2 counters, the layout lwIP is compiled with.
 */
#include "lwip/stats.h"

#if !LWIP_STATS || !TCP_STATS || !MIB2_STATS
#error "lwIP must keep TCP and MIB2 statistics (CONFIG_LWIP_STATS, MIB2_STATS)"
#endif
