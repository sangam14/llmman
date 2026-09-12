defmodule Dashboard.ClusterClient do
  @moduledoc """
  Communicates with Brigade microVM cluster nodes (`localhost:9600/status`)
  and local `llmman` Firecracker runtime records.
  """

  @default_brigade_url "http://localhost:9600"

  def get_cluster_status(url \\ @default_brigade_url) do
    case Req.get("#{url}/status", retry: false) do
      {:ok, %{status: 200, body: body}} when is_map(body) ->
        local_vms = read_local_llmman_vms()

        merged_vms =
          if Enum.empty?(local_vms) do
            body["vms"] || []
          else
            local_vms ++
              Enum.reject(body["vms"] || [], fn vm ->
                Enum.any?(local_vms, &(&1["uid"] == vm["uid"]))
              end)
          end

        total_vcpus = Enum.reduce(body["hosts"] || [], 0, fn h, acc -> acc + (h["vcpus"] || 0) end)
        used_vcpus = Enum.reduce(merged_vms, 0, fn vm, acc -> acc + (if vm["status"] == "running", do: vm["vcpus"] || 0, else: 0) end)
        total_mem = Enum.reduce(body["hosts"] || [], 0, fn h, acc -> acc + (h["memory_mb"] || 0) end)
        used_mem = Enum.reduce(merged_vms, 0, fn vm, acc -> acc + (if vm["status"] == "running", do: vm["memory_mb"] || 0, else: 0) end)

        data =
          body
          |> Map.put("vms", merged_vms)
          |> Map.put("total_vcpus", total_vcpus)
          |> Map.put("used_vcpus", used_vcpus)
          |> Map.put("total_mem_mb", total_mem)
          |> Map.put("used_mem_mb", used_mem)

        {:ok, data}

      _ ->
        {:ok, real_system_status()}
    end
  end

  def real_system_status do
    local_vms = read_local_llmman_vms()
    hostname = detect_hostname()
    total_vcpus = :erlang.system_info(:logical_processors_online)
    total_mem_mb = detect_total_memory_mb()
    host_ip = detect_host_ip()
    arch = detect_architecture()

    used_vcpus =
      Enum.reduce(local_vms, 0, fn vm, acc ->
        acc + (if vm["status"] == "running", do: vm["vcpus"] || 0, else: 0)
      end)

    used_mem_mb =
      Enum.reduce(local_vms, 0, fn vm, acc ->
        acc + (if vm["status"] == "running", do: vm["memory_mb"] || 0, else: 0)
      end)

    host = %{
      "id" => hostname,
      "status" => "online",
      "vcpus" => total_vcpus,
      "vcpus_used" => used_vcpus,
      "memory_mb" => total_mem_mb,
      "memory_mb_used" => used_mem_mb,
      "active_vms" => length(local_vms),
      "warm_pool" => 0,
      "ip" => host_ip,
      "arch" => arch
    }

    cluster = %{
      "name" => "llmman-#{hostname}",
      "quorum" => true,
      "nodes_online" => 1,
      "min_size" => 1,
      "network" => "Host Network (#{host_ip})",
      "consensus" => "Local Host Engine (#{arch})"
    }

    events = [
      %{
        "time" => time_now(),
        "type" => "SYSTEM",
        "msg" => "Host #{hostname} online (#{total_vcpus} vCPUs, #{round(total_mem_mb / 1024)} GB RAM, #{arch})"
      }
      | Enum.map(local_vms, fn vm ->
          %{
            "time" => time_now(),
            "type" => "VM",
            "msg" => "MicroVM #{vm["uid"]} (#{vm["name"]}) active on PID #{vm["pid"]}"
          }
        end)
    ]

    %{
      "cluster" => cluster,
      "hosts" => [host],
      "vms" => local_vms,
      "snapshots" => read_local_snapshots(),
      "proxy" => %{
        "tokens_processed" => 0,
        "requests_proxied" => 0,
        "spend_usd" => 0.0,
        "budget_limit_usd" => 0.0,
        "active_keys" => detect_active_keys(),
        "rate_limit_status" => "Normal"
      },
      "events" => events,
      "total_vcpus" => total_vcpus,
      "used_vcpus" => used_vcpus,
      "total_mem_mb" => total_mem_mb,
      "used_mem_mb" => used_mem_mb
    }
  end

  def read_local_llmman_vms do
    dirs = [
      Path.expand("~/Library/Application Support/llmman/vms"),
      Path.expand("~/.local/share/llmman/vms"),
      "/tmp/llmman/vms"
    ]

    hostname = detect_hostname()

    Enum.find_value(dirs, [], fn dir ->
      if File.dir?(dir) do
        dir
        |> File.ls!()
        |> Enum.filter(&String.ends_with?(&1, ".json"))
        |> Enum.map(fn file ->
          path = Path.join(dir, file)
          with {:ok, content} <- File.read(path),
               {:ok, data} <- Jason.decode(content) do
            pid = data["pid"]
            is_alive = is_pid_alive?(pid)

            if is_alive do
              %{
                "uid" => data["id"],
                "name" => data["model"] || "microvm",
                "host" => hostname,
                "pid" => pid,
                "vcpus" => data["vcpus"] || 4,
                "memory_mb" => data["memory_mb"] || 8192,
                "status" => String.downcase(data["status"] || "running"),
                "kernel" => data["kernel"] || "6.18.45-agentkernel",
                "ip" => data["ip"] || "172.16.0.2",
                "tap" => data["tap"] || "vmtap-0",
                "socket_path" => data["socket_path"]
              }
            else
              nil
            end
          else
            _ -> nil
          end
        end)
        |> Enum.reject(&is_nil/1)
      end
    end) || []
  end

  defp is_pid_alive?(pid) when is_integer(pid) do
    case System.cmd("kill", ["-0", "#{pid}"], stderr_to_stdout: true) do
      {_, 0} -> true
      _ -> false
    end
  rescue
    _ -> true
  end
  defp is_pid_alive?(_), do: true

  defp detect_hostname do
    case System.cmd("hostname", ["-s"]) do
      {name, 0} -> String.trim(name)
      _ -> "localhost"
    end
  rescue
    _ -> "localhost"
  end

  defp detect_total_memory_mb do
    case System.cmd("sysctl", ["-n", "hw.memsize"]) do
      {val, 0} ->
        case Integer.parse(String.trim(val)) do
          {bytes, ""} -> div(bytes, 1024 * 1024)
          _ -> 16384
        end
      _ ->
        16384
    end
  rescue
    _ -> 16384
  end

  defp detect_host_ip do
    case :inet.getifaddrs() do
      {:ok, ifaddrs} ->
        Enum.find_value(ifaddrs, "127.0.0.1", fn {_name, opts} ->
          flags = Keyword.get(opts, :flags, [])
          if :up in flags and :loopback not in flags do
            addrs =
              Keyword.get_values(opts, :addr)
              |> Enum.filter(&(is_tuple(&1) and tuple_size(&1) == 4))

            case addrs do
              [first | _] -> :inet.ntoa(first) |> to_string()
              _ -> nil
            end
          else
            nil
          end
        end)
      _ ->
        "127.0.0.1"
    end
  rescue
    _ -> "127.0.0.1"
  end

  defp detect_architecture do
    os =
      case :os.type() do
        {:unix, :darwin} -> "macOS"
        {:unix, :linux} -> "Linux"
        {:win32, _} -> "Windows"
        other -> inspect(other)
      end

    arch =
      case System.cmd("uname", ["-m"]) do
        {m, 0} -> String.trim(m)
        _ -> to_string(:erlang.system_info(:system_architecture))
      end

    "#{os} #{arch}"
  rescue
    _ -> "Darwin arm64"
  end

  defp detect_active_keys do
    ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "HF_TOKEN"]
    |> Enum.filter(fn k ->
      case System.get_env(k) do
        nil -> false
        "" -> false
        _ -> true
      end
    end)
  end

  defp read_local_snapshots do
    dirs = [
      Path.expand("~/Library/Application Support/llmman/snapshots"),
      Path.expand("~/.local/share/llmman/snapshots"),
      "/tmp/llmman/snapshots"
    ]

    Enum.find_value(dirs, [], fn dir ->
      if File.dir?(dir) do
        dir
        |> File.ls!()
        |> Enum.filter(&String.ends_with?(&1, ".json"))
        |> Enum.map(fn file ->
          with {:ok, content} <- File.read(Path.join(dir, file)),
               {:ok, data} <- Jason.decode(content) do
            %{
              "name" => data["name"] || file,
              "vm" => data["vm_id"] || data["vm"] || "microvm",
              "size_mb" => data["size_mb"] || 8192,
              "created_at" => data["created_at"] || DateTime.utc_now() |> DateTime.to_iso8601(),
              "status" => data["status"] || "ready"
            }
          else
            _ -> nil
          end
        end)
        |> Enum.reject(&is_nil/1)
      end
    end) || []
  rescue
    _ -> []
  end

  defp time_now do
    Calendar.strftime(DateTime.utc_now(), "%H:%M:%S")
  end

  def simulated_status do
    %{
      "cluster" => %{
        "name" => "llmman-brigade-mesh",
        "quorum" => true,
        "nodes_online" => 3,
        "min_size" => 1,
        "network" => "CNI ptp/bridge mesh (172.16.0.0/16)",
        "consensus" => "Raft v2.4 (Sub-millisecond heartbeat)"
      },
      "hosts" => [
        %{
          "id" => "node-alpha",
          "status" => "online",
          "vcpus" => 32,
          "vcpus_used" => 8,
          "memory_mb" => 65536,
          "memory_mb_used" => 16384,
          "active_vms" => 2,
          "warm_pool" => 3,
          "ip" => "192.168.1.101",
          "arch" => "x86_64 / Apple Silicon KVM"
        },
        %{
          "id" => "node-beta",
          "status" => "online",
          "vcpus" => 32,
          "vcpus_used" => 4,
          "memory_mb" => 65536,
          "memory_mb_used" => 8192,
          "active_vms" => 1,
          "warm_pool" => 3,
          "ip" => "192.168.1.102",
          "arch" => "x86_64 AMD EPYC"
        },
        %{
          "id" => "node-gamma",
          "status" => "online",
          "vcpus" => 16,
          "vcpus_used" => 2,
          "memory_mb" => 32768,
          "memory_mb_used" => 4096,
          "active_vms" => 0,
          "warm_pool" => 3,
          "ip" => "192.168.1.103",
          "arch" => "x86_64 Intel Xeon"
        }
      ],
      "vms" => [
        %{
          "uid" => "vm-01h8x9a",
          "name" => "llama-3-8b-instruct",
          "host" => "node-alpha",
          "pid" => 48120,
          "vcpus" => 4,
          "memory_mb" => 8192,
          "status" => "running",
          "kernel" => "6.18.45-agentkernel",
          "ip" => "172.16.0.2",
          "tap" => "vmtap-0"
        },
        %{
          "uid" => "vm-01h8x9b",
          "name" => "vllm-deepseek-coder",
          "host" => "node-alpha",
          "pid" => 48124,
          "vcpus" => 4,
          "memory_mb" => 8192,
          "status" => "running",
          "kernel" => "6.18.45-agentkernel",
          "ip" => "172.16.0.3",
          "tap" => "vmtap-1"
        },
        %{
          "uid" => "vm-01h8x9c",
          "name" => "qwen-2.5-7b",
          "host" => "node-beta",
          "pid" => 48130,
          "vcpus" => 2,
          "memory_mb" => 4096,
          "status" => "paused",
          "kernel" => "6.18.45-agentkernel",
          "ip" => "172.16.0.4",
          "tap" => "vmtap-2"
        }
      ],
      "snapshots" => [
        %{
          "name" => "llama3-post-warmup",
          "vm" => "llama-3-8b-instruct",
          "size_mb" => 8200,
          "created_at" => "2026-09-12T21:30:00Z",
          "status" => "ready"
        },
        %{
          "name" => "deepseek-coder-checkpoint-01",
          "vm" => "vllm-deepseek-coder",
          "size_mb" => 8192,
          "created_at" => "2026-09-12T22:15:00Z",
          "status" => "ready"
        }
      ],
      "proxy" => %{
        "tokens_processed" => 1_428_950,
        "requests_proxied" => 3_410,
        "spend_usd" => 2.85,
        "budget_limit_usd" => 25.00,
        "active_keys" => ["OPENAI_API_KEY", "ANTHROPIC_API_KEY", "HF_TOKEN"],
        "rate_limit_status" => "Healthy (0 throttle events)"
      },
      "events" => [
        %{"time" => "22:48:59", "type" => "BOOT", "msg" => "MicroVM vm-0000ea14 booted in 51ms (kernel: 6.18.45-agentkernel)"},
        %{"time" => "22:47:24", "type" => "BOOT", "msg" => "MicroVM vm-0000e683 booted in 53ms (vllm-deepseek-coder)"},
        %{"time" => "22:45:12", "type" => "POOL", "msg" => "VmPool replenished to 3 warm instances (<100ms SLA)"},
        %{"time" => "22:30:00", "type" => "SNAPSHOT", "msg" => "Full-state memory snapshot saved (8.2 GB)"}
      ]
    }
  end
end
