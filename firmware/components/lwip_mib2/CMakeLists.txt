# Turns on lwIP's SNMP MIB2 counters, which hold the TCP retransmit count
# (`lwip_stats.mib2.tcpretranssegs`). ESP-IDF's Kconfig has no option for
# them. The definition is public on the lwip library, so lwIP itself, every
# component that uses lwIP, and the Rust bindings (generated with the
# definitions the application links with) all see the same `struct stats_`.
idf_component_register(REQUIRES lwip)

idf_component_get_property(lwip lwip COMPONENT_LIB)
target_compile_definitions(${lwip} PUBLIC MIB2_STATS=1)
