defmodule Dashboard.ClusterClientTest do
  use ExUnit.Case, async: true
  alias Dashboard.ClusterClient

  test "get_cluster_status returns expected structural keys" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    assert is_map(state)
    assert Map.has_key?(state, "hosts")
    assert Map.has_key?(state, "vms")
    assert Map.has_key?(state, "snapshots")
    assert Map.has_key?(state, "proxy")
    assert Map.has_key?(state, "events")
    assert Map.has_key?(state, "total_vcpus")
    assert Map.has_key?(state, "used_vcpus")
  end

  test "get_cluster_status contains valid host records" do
    assert {:ok, state} = ClusterClient.get_cluster_status()
    hosts = state["hosts"]

    assert is_list(hosts)
    assert length(hosts) >= 1

    first_host = hd(hosts)
    assert Map.has_key?(first_host, "id")
    assert Map.has_key?(first_host, "status")
    assert Map.has_key?(first_host, "vcpus")
    assert Map.has_key?(first_host, "memory_mb")
  end

  test "get_cluster_status proxy metrics are valid" do
    assert {:ok, state} = ClusterClient.get_cluster_status()
    proxy = state["proxy"]

    assert is_map(proxy)
    assert is_integer(proxy["tokens_processed"])
    assert is_integer(proxy["requests_proxied"])
    assert is_list(proxy["active_keys"])
  end

  test "read_local_llmman_vms returns list" do
    vms = ClusterClient.read_local_llmman_vms()
    assert is_list(vms)
  end
end
