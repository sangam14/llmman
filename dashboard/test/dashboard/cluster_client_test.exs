defmodule Dashboard.ClusterClientTest do
  use ExUnit.Case, async: true
  alias Dashboard.ClusterClient

  # ---------------------------------------------------------------------------
  # Structural keys
  # ---------------------------------------------------------------------------

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
    assert Map.has_key?(state, "total_mem_mb")
    assert Map.has_key?(state, "used_mem_mb")
  end

  # ---------------------------------------------------------------------------
  # Host records
  # ---------------------------------------------------------------------------

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

  test "every host has a non-empty id and positive vcpus" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    for host <- state["hosts"] do
      assert is_binary(host["id"]) and byte_size(host["id"]) > 0
      assert is_integer(host["vcpus"]) and host["vcpus"] > 0
      assert is_integer(host["memory_mb"]) and host["memory_mb"] > 0
    end
  end

  # ---------------------------------------------------------------------------
  # VM records
  # ---------------------------------------------------------------------------

  test "every VM has required fields with correct types" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    for vm <- state["vms"] do
      assert is_binary(vm["uid"])
      assert is_binary(vm["name"])
      assert is_binary(vm["status"])
      assert vm["status"] in ["running", "paused", "stopped"]
      assert is_integer(vm["vcpus"]) and vm["vcpus"] > 0
      assert is_integer(vm["memory_mb"]) and vm["memory_mb"] > 0
    end
  end

  # ---------------------------------------------------------------------------
  # Resource utilization calculations
  # ---------------------------------------------------------------------------

  test "used_vcpus is the sum of running VM vcpus" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    expected =
      state["vms"]
      |> Enum.filter(&(&1["status"] == "running"))
      |> Enum.reduce(0, fn vm, acc -> acc + (vm["vcpus"] || 0) end)

    assert state["used_vcpus"] == expected
  end

  test "used_mem_mb is the sum of running VM memory" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    expected =
      state["vms"]
      |> Enum.filter(&(&1["status"] == "running"))
      |> Enum.reduce(0, fn vm, acc -> acc + (vm["memory_mb"] || 0) end)

    assert state["used_mem_mb"] == expected
  end

  test "total_vcpus is the sum across all hosts" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    expected =
      state["hosts"]
      |> Enum.reduce(0, fn h, acc -> acc + (h["vcpus"] || 0) end)

    assert state["total_vcpus"] == expected
  end

  test "total_mem_mb is the sum across all hosts" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    expected =
      state["hosts"]
      |> Enum.reduce(0, fn h, acc -> acc + (h["memory_mb"] || 0) end)

    assert state["total_mem_mb"] == expected
  end

  # ---------------------------------------------------------------------------
  # Proxy metrics
  # ---------------------------------------------------------------------------

  test "get_cluster_status proxy metrics are valid" do
    assert {:ok, state} = ClusterClient.get_cluster_status()
    proxy = state["proxy"]

    assert is_map(proxy)
    assert is_integer(proxy["tokens_processed"])
    assert is_integer(proxy["requests_proxied"])
    assert is_list(proxy["active_keys"])
  end

  test "proxy budget_limit_usd is non-negative" do
    assert {:ok, state} = ClusterClient.get_cluster_status()
    proxy = state["proxy"]
    assert is_number(proxy["budget_limit_usd"]) and proxy["budget_limit_usd"] >= 0
  end

  # ---------------------------------------------------------------------------
  # Snapshots
  # ---------------------------------------------------------------------------

  test "snapshot records have required fields" do
    assert {:ok, state} = ClusterClient.get_cluster_status()

    for snap <- state["snapshots"] do
      assert is_binary(snap["name"])
      assert is_binary(snap["vm"])
      assert is_integer(snap["size_mb"])
      assert snap["status"] in ["ready", "creating", "failed"]
    end
  end

  # ---------------------------------------------------------------------------
  # Local VM reading
  # ---------------------------------------------------------------------------

  test "read_local_llmman_vms returns list" do
    vms = ClusterClient.read_local_llmman_vms()
    assert is_list(vms)
  end

  test "read_local_llmman_vms with temp dir containing valid JSON" do
    tmp_dir = Path.join(System.tmp_dir!(), "llmman_test_vms_#{:rand.uniform(100_000)}")
    File.mkdir_p!(tmp_dir)

    record = %{
      "id" => "vm-test-001",
      "pid" => System.pid() |> String.to_integer(),
      "status" => "RUNNING",
      "kernel" => "6.18.45-agentkernel",
      "vcpus" => 2,
      "memory_mb" => 4096,
      "ip" => "172.16.0.99",
      "tap" => "vmtap-test",
      "model" => "test-model",
      "started_at" => DateTime.utc_now() |> DateTime.to_iso8601(),
      "socket_path" => "/tmp/test.sock"
    }

    File.write!(Path.join(tmp_dir, "vm-test-001.json"), Jason.encode!(record))

    # The function reads from hardcoded paths, so this is mainly a smoke
    # test to ensure the JSON parsing logic doesn't crash.
    vms = ClusterClient.read_local_llmman_vms()
    assert is_list(vms)
  after
    tmp_dir = Path.join(System.tmp_dir!(), "llmman_test_vms_*")
    for dir <- Path.wildcard(tmp_dir), do: File.rm_rf!(dir)
  end

  # ---------------------------------------------------------------------------
  # Simulated status (unit test the mock data shape)
  # ---------------------------------------------------------------------------

  test "simulated_status has correct structure" do
    data = ClusterClient.simulated_status()

    assert is_map(data["cluster"])
    assert data["cluster"]["quorum"] == true
    assert is_integer(data["cluster"]["nodes_online"])

    assert is_list(data["hosts"])
    assert is_list(data["vms"])
    assert is_list(data["snapshots"])
    assert is_list(data["events"])
    assert is_map(data["proxy"])
  end

  test "simulated_status VM statuses are valid enum values" do
    data = ClusterClient.simulated_status()

    for vm <- data["vms"] do
      assert vm["status"] in ["running", "paused", "stopped"]
    end
  end

  test "simulated events have time, type, and msg" do
    data = ClusterClient.simulated_status()

    for event <- data["events"] do
      assert is_binary(event["time"])
      assert is_binary(event["type"])
      assert is_binary(event["msg"])
    end
  end
end
