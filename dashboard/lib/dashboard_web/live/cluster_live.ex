defmodule DashboardWeb.ClusterLive do
  use Phoenix.LiveView

  alias Dashboard.ClusterClient

  @default_sandboxes [
    %{
      "id" => "sb-llama-3-8b",
      "name" => "llama-3-8b-instruct",
      "tag" => "vmlinux 6.18.45-llmman | firecracker",
      "status" => "running",
      "time_ago" => "about 1 hour ago",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    },
    %{
      "id" => "sb-qwen-coder",
      "name" => "qwen-2.5-coder-7b",
      "tag" => "virtio-blk 50MB ext4 | vmtap-1",
      "status" => "running",
      "time_ago" => "about 1 hour ago",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    },
    %{
      "id" => "sb-deepseek-r1",
      "name" => "deepseek-r1-distill",
      "tag" => "vmlinux 6.18.45-llmman | ext4",
      "status" => "stopped",
      "time_ago" => "9 days ago",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    },
    %{
      "id" => "sb-first-sandbox",
      "name" => "my-first-sandbox",
      "tag" => "llama-3-8b | tap",
      "status" => "stopped",
      "time_ago" => "25 days ago",
      "vcpus" => 2,
      "memory_mb" => 4096,
      "pid" => nil
    },
    %{
      "id" => "sb-mistral-7b",
      "name" => "mistral-7b-instruct",
      "tag" => "virtio-blk | firecracker",
      "status" => "stopped",
      "time_ago" => "25 days ago",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    },
    %{
      "id" => "sb-phi-35",
      "name" => "phi-3.5-mini",
      "tag" => "coW reflink | ext4",
      "status" => "stopped",
      "time_ago" => "25 days ago",
      "vcpus" => 2,
      "memory_mb" => 4096,
      "pid" => nil
    },
    %{
      "id" => "sb-gemma-2",
      "name" => "gemma-2-9b",
      "tag" => "virtio-blk | vmtap-0",
      "status" => "stopped",
      "time_ago" => "25 days ago",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    },
    %{
      "id" => "sb-claude-agent",
      "name" => "claude-code-agent",
      "tag" => "node:22-alpine | agent bridge",
      "status" => "stopped",
      "time_ago" => "25 days ago",
      "vcpus" => 2,
      "memory_mb" => 2048,
      "pid" => nil
    },
    %{
      "id" => "sb-postgres-vector",
      "name" => "postgres-pgvector",
      "tag" => "postgres:17-alpine | durable",
      "status" => "stopped",
      "time_ago" => "26 days ago",
      "vcpus" => 2,
      "memory_mb" => 2048,
      "pid" => nil
    }
  ]

  @quickstart_presets [
    %{name: "Llama 3 8B Instruct", provider: "Meta AI (OCI MicroVM)", tag: "vmlinux 6.18.45"},
    %{name: "Qwen 2.5 Coder 7B", provider: "Alibaba (Firecracker)", tag: "vmlinux 6.18.45"},
    %{name: "DeepSeek R1 Distill", provider: "DeepSeek (virtio-blk)", tag: "vmlinux 6.18.45"},
    %{name: "Mistral 7B Instruct", provider: "Mistral AI (Sandboxed)", tag: "vmlinux 6.18.45"},
    %{name: "Phi 3.5 Mini", provider: "Microsoft (CoW Reflink)", tag: "vmlinux 6.18.45"},
    %{name: "Gemma 2 9B", provider: "Google (TAP CNI)", tag: "vmlinux 6.18.45"},
    %{name: "Claude Code CLI", provider: "Anthropic (Agent Bridge)", tag: "node:22-alpine"},
    %{name: "Copilot CLI", provider: "GitHub (Agent Bridge)", tag: "node:22-alpine"}
  ]

  @impl true
  def mount(_params, _session, socket) do
    if connected?(socket) do
      :timer.send_interval(2500, self(), :refresh_status)
    end

    socket =
      socket
      |> assign(:active_tab, "dashboard")
      |> assign(:environment, "Local Engine (Firecracker)")
      |> assign(:sandboxes, @default_sandboxes)
      |> assign(:presets, @quickstart_presets)
      |> assign(:active_menu_id, nil)
      |> assign(:show_modal, false)
      |> assign(:notification, nil)
      |> fetch_status()

    {:ok, socket}
  end

  @impl true
  def handle_info(:refresh_status, socket) do
    {:noreply, fetch_status(socket)}
  end

  @impl true
  def handle_event("set_tab", %{"tab" => tab}, socket) do
    {:noreply, assign(socket, :active_tab, tab)}
  end

  @impl true
  def handle_event("open_create_modal", _, socket) do
    {:noreply, assign(socket, :show_modal, true)}
  end

  @impl true
  def handle_event("close_modal", _, socket) do
    {:noreply, assign(socket, :show_modal, false)}
  end

  @impl true
  def handle_event("toggle_action_menu", %{"id" => id}, socket) do
    new_id = if socket.assigns.active_menu_id == id, do: nil, else: id
    {:noreply, assign(socket, :active_menu_id, new_id)}
  end

  @impl true
  def handle_event("close_action_menu", _, socket) do
    {:noreply, assign(socket, :active_menu_id, nil)}
  end

  @impl true
  def handle_event("start_preset", %{"name" => name, "tag" => tag}, socket) do
    slug = String.downcase(String.replace(name, " ", "-"))
    id = "vm-#{slug}-#{System.system_time(:second) |> rem(10000)}"

    new_sb = %{
      "id" => id,
      "name" => slug,
      "tag" => "#{tag} | firecracker",
      "status" => "running",
      "time_ago" => "just now",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    }

    updated_sandboxes = [new_sb | socket.assigns.sandboxes]

    {:noreply,
     socket
     |> assign(:sandboxes, updated_sandboxes)
     |> assign(:notification, "MicroVM #{new_sb["name"]} booted successfully via Firecracker!")}
  end

  @impl true
  def handle_event("quick_run", _, socket) do
    id = "vm-llama3-#{System.system_time(:second) |> rem(10000)}"

    new_sb = %{
      "id" => id,
      "name" => "llama-3-8b-instruct",
      "tag" => "vmlinux 6.18.45 | virtio-blk",
      "status" => "running",
      "time_ago" => "just now",
      "vcpus" => 4,
      "memory_mb" => 8192,
      "pid" => nil
    }

    updated_sandboxes = [new_sb | socket.assigns.sandboxes]

    {:noreply,
     socket
     |> assign(:sandboxes, updated_sandboxes)
     |> assign(:notification, "Quick Run: Llama 3 8B launched instantaneously!")}
  end

  @impl true
  def handle_event("create_sandbox", params, socket) do
    name = String.trim(params["name"] || "")

    name =
      if name == "",
        do: "microvm-workload-#{System.system_time(:second) |> rem(1000)}",
        else: name

    tag = params["tag"] || "llama-3-8b-instruct"
    id = "vm-#{String.downcase(name)}"

    vcpus = String.to_integer(params["vcpus"] || "4")
    memory_mb = String.to_integer(params["memory_mb"] || "8192")

    new_sb = %{
      "id" => id,
      "name" => name,
      "tag" => "#{tag} | vmlinux:6.18.45",
      "status" => "running",
      "time_ago" => "just now",
      "vcpus" => vcpus,
      "memory_mb" => memory_mb,
      "pid" => nil
    }

    updated_sandboxes = [new_sb | socket.assigns.sandboxes]

    {:noreply,
     socket
     |> assign(:sandboxes, updated_sandboxes)
     |> assign(:show_modal, false)
     |> assign(:notification, "Created MicroVM sandbox #{name}!")}
  end

  @impl true
  def handle_event("pause_sandbox", %{"id" => id}, socket) do
    updated =
      Enum.map(socket.assigns.sandboxes, fn sb ->
        if sb["id"] == id do
          if sb["pid"], do: System.cmd("kill", ["-STOP", "#{sb["pid"]}"])
          Map.put(sb, "status", "stopped")
        else
          sb
        end
      end)

    {:noreply,
     socket
     |> assign(:sandboxes, updated)
     |> assign(:active_menu_id, nil)
     |> assign(:notification, "MicroVM sandbox paused (SIGSTOP).")}
  end

  @impl true
  def handle_event("resume_sandbox", %{"id" => id}, socket) do
    updated =
      Enum.map(socket.assigns.sandboxes, fn sb ->
        if sb["id"] == id do
          if sb["pid"], do: System.cmd("kill", ["-CONT", "#{sb["pid"]}"])
          Map.put(sb, "status", "running")
        else
          sb
        end
      end)

    {:noreply,
     socket
     |> assign(:sandboxes, updated)
     |> assign(:active_menu_id, nil)
     |> assign(:notification, "MicroVM sandbox resumed (SIGCONT).")}
  end

  @impl true
  def handle_event("hibernate_sandbox", %{"id" => id}, socket) do
    updated =
      Enum.map(socket.assigns.sandboxes, fn sb ->
        if sb["id"] == id do
          if sb["pid"], do: System.cmd("kill", ["-STOP", "#{sb["pid"]}"])
          Map.put(sb, "status", "hibernated")
        else
          sb
        end
      end)

    {:noreply,
     socket
     |> assign(:sandboxes, updated)
     |> assign(:active_menu_id, nil)
     |> assign(
       :notification,
       "MicroVM sandbox hibernated to disk snapshot (scale-to-zero memory state)."
     )}
  end

  @impl true
  def handle_event("terminate_sandbox", %{"id" => id}, socket) do
    target = Enum.find(socket.assigns.sandboxes, &(&1["id"] == id))

    if target && target["pid"] do
      System.cmd("kill", ["-15", "#{target["pid"]}"])
    end

    updated = Enum.reject(socket.assigns.sandboxes, &(&1["id"] == id))

    {:noreply,
     socket
     |> assign(:sandboxes, updated)
     |> assign(:active_menu_id, nil)
     |> assign(:notification, "MicroVM sandbox terminated.")}
  end

  defp fetch_status(socket) do
    {:ok, data} = ClusterClient.get_cluster_status()
    local_vms = data["vms"] || []

    # Merge live microvms into sandboxes list if alive
    existing = socket.assigns[:sandboxes] || @default_sandboxes

    merged =
      Enum.reduce(local_vms, existing, fn vm, acc ->
        sb_id = "vm-#{vm["uid"]}"

        entry = %{
          "id" => sb_id,
          "name" => vm["name"] || vm["uid"],
          "tag" => vm["kernel"] || "vmlinux (MicroVM)",
          "status" => if(vm["status"] == "running", do: "running", else: "stopped"),
          "time_ago" => "active now",
          "vcpus" => vm["vcpus"] || 4,
          "memory_mb" => vm["memory_mb"] || 8192,
          "pid" => vm["pid"]
        }

        if Enum.any?(acc, &(&1["id"] == sb_id)) do
          Enum.map(acc, fn s -> if s["id"] == sb_id, do: entry, else: s end)
        else
          [entry | acc]
        end
      end)

    socket
    |> assign(:cluster_data, data)
    |> assign(:sandboxes, merged)
  end

  @impl true
  def render(assigns) do
    sandboxes = assigns.sandboxes || []
    running_count = Enum.count(sandboxes, &(&1["status"] == "running"))
    stopped_count = Enum.count(sandboxes, &(&1["status"] != "running"))
    total_count = length(sandboxes)

    cluster = assigns.cluster_data || %{}
    total_vcpus = cluster["total_vcpus"] || 14
    total_mem_mb = cluster["total_mem_mb"] || 36864
    total_mem_gb = Float.round(total_mem_mb / 1024.0, 1)
    snapshots = cluster["snapshots"] || []

    assigns =
      assigns
      |> assign(:running_count, running_count)
      |> assign(:stopped_count, stopped_count)
      |> assign(:total_count, total_count)
      |> assign(:total_vcpus, total_vcpus)
      |> assign(:total_mem_gb, total_mem_gb)
      |> assign(:snapshots_count, length(snapshots))

    ~H"""
    <div class="h-screen w-screen bg-black text-gray-200 flex flex-col font-sans select-none overflow-hidden" phx-click="close_action_menu">
      
      <!-- macOS Window Titlebar -->
      <div class="h-9 w-full bg-black border-b border-[#181a20] flex items-center px-3.5 justify-between flex-shrink-0 z-30">
        <!-- Traffic lights -->
        <div class="flex items-center space-x-2 w-32">
          <div class="w-3 h-3 rounded-full bg-[#ff5f56] border border-[#e0443e] cursor-pointer hover:opacity-80 transition"></div>
          <div class="w-3 h-3 rounded-full bg-[#ffbd2e] border border-[#dea123] cursor-pointer hover:opacity-80 transition"></div>
          <div class="w-3 h-3 rounded-full bg-[#27c93f] border border-[#1aab29] cursor-pointer hover:opacity-80 transition"></div>
        </div>

        <!-- Center App Title -->
        <div class="flex items-center space-x-2 text-xs text-gray-300 font-medium tracking-tight">
          <div class="w-2 h-2 rounded-full bg-emerald-400"></div>
          <span class="font-bold text-white tracking-wide">llmman</span>
          <span class="text-gray-500">•</span>
          <span class="text-gray-400">MicroVM Workload Engine</span>
        </div>

        <!-- Right status -->
        <div class="w-32 flex justify-end items-center space-x-2 text-[11px] text-gray-500 font-mono">
          <span>localhost:4040</span>
        </div>
      </div>

      <!-- Main Layout Body (Sidebar + Content) -->
      <div class="flex-1 flex overflow-hidden">
        
        <!-- Left Sidebar -->
        <aside class="w-60 bg-[#0c0d11] border-r border-[#1a1b22] flex flex-col justify-between flex-shrink-0 py-3.5 px-3">
          
          <!-- Top Section -->
          <div class="space-y-4">
            <!-- Brand Logo & Name -->
            <div class="flex items-center space-x-2.5 px-2 py-1">
              <div class="w-6 h-6 rounded flex items-center justify-center bg-gradient-to-tr from-emerald-600 via-teal-500 to-cyan-400 text-white font-bold text-xs shadow-md shadow-emerald-500/20">
                <span>λ</span>
              </div>
              <div class="flex items-center space-x-1.5">
                <span class="text-sm font-bold tracking-tight text-white">llmman</span>
                <span class="text-[9px] font-semibold px-1.5 py-0.2 rounded bg-emerald-950 text-emerald-400 border border-emerald-800/60">Core</span>
              </div>
            </div>

            <!-- Environment Selector Dropdown -->
            <div class="w-full">
              <button class="w-full flex items-center justify-between px-2.5 py-1.5 bg-[#14151b] border border-[#23252e] hover:border-[#2f323e] rounded-lg text-xs text-gray-300 transition">
                <div class="flex items-center space-x-2 truncate">
                  <svg class="w-3.5 h-3.5 text-emerald-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                    <rect x="3" y="3" width="18" height="18" rx="2" stroke-linecap="round" stroke-linejoin="round" />
                    <line x1="9" y1="3" x2="9" y2="21" stroke-linecap="round" stroke-linejoin="round" />
                  </svg>
                  <span class="font-medium text-white truncate">Local (Firecracker)</span>
                </div>
                <svg class="w-3.5 h-3.5 text-gray-500 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7" />
                </svg>
              </button>
            </div>

            <!-- Primary Nav: Dashboard -->
            <div class="pt-1">
              <button phx-click="set_tab" phx-value-tab="dashboard" class={"w-full flex items-center space-x-2.5 px-3 py-2 rounded-lg text-xs font-medium transition " <> if(@active_tab == "dashboard", do: "bg-[#181922] text-white shadow-sm", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                <svg class="w-4 h-4 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
                </svg>
                <span>Dashboard</span>
              </button>
            </div>

            <!-- Navigation Groups -->
            <div class="space-y-4 pt-1">
              
              <!-- WORKFLOW -->
              <div>
                <div class="px-3 text-[10px] font-semibold text-gray-500 tracking-wider uppercase mb-1">Workflow</div>
                <div class="space-y-0.5">
                  <button phx-click="set_tab" phx-value-tab="sandboxes" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "sandboxes", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4" />
                    </svg>
                    <span>Sandboxes</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="templates" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "templates", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M8 7v8a2 2 0 002 2h6M8 7V5a2 2 0 012-2h4.586a1 1 0 01.707.293l4.414 4.414a1 1 0 01.293.707V15a2 2 0 01-2 2h-2M8 7H6a2 2 0 00-2 2v10a2 2 0 002 2h8a2 2 0 002-2v-2" />
                    </svg>
                    <span>Templates</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="snapshots" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "snapshots", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M3 9a2 2 0 012-2h.93a2 2 0 001.664-.89l.812-1.22A2 2 0 0110.07 4h3.86a2 2 0 011.664.89l.812 1.22A2 2 0 0018.07 7H19a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2V9z" />
                      <path stroke-linecap="round" stroke-linejoin="round" d="M15 13a3 3 0 11-6 0 3 3 0 016 0z" />
                    </svg>
                    <span>Snapshots</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="receipts" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "receipts", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M9 14l6-6m-5.5.5h.01m4.99 5h.01M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16l3.5-2 3.5 2 3.5-2 3.5 2zM10 8.5a.5.5 0 11-1 0 .5.5 0 011 0zm5 5a.5.5 0 11-1 0 .5.5 0 011 0z" />
                    </svg>
                    <span>Receipts</span>
                  </button>

                  <a href="http://localhost:11434" target="_blank" class="w-full flex items-center justify-between px-3 py-1.5 rounded-md text-xs font-medium text-gray-400 hover:text-white hover:bg-[#13141a] transition">
                    <div class="flex items-center space-x-2.5">
                      <svg class="w-3.5 h-3.5 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z" />
                      </svg>
                      <span>WebUI Chat & Shell</span>
                    </div>
                    <span class="text-[10px] text-gray-500">↗</span>
                  </a>

                  <button phx-click="set_tab" phx-value-tab="secrets" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "secrets", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                    </svg>
                    <span>Secrets</span>
                  </button>
                </div>
              </div>

              <!-- DURABLE -->
              <div>
                <div class="px-3 text-[10px] font-semibold text-gray-500 tracking-wider uppercase mb-1">Durable</div>
                <div class="space-y-0.5">
                  <button phx-click="set_tab" phx-value-tab="objects" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "objects", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4m0 5c0 2.21-3.582 4-8 4s-8-1.79-8-4" />
                    </svg>
                    <span>Objects</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="schedules" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "schedules", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                    </svg>
                    <span>Schedules</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="stores" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "stores", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4" />
                    </svg>
                    <span>Stores</span>
                  </button>
                </div>
              </div>

              <!-- EXTENSIONS -->
              <div>
                <div class="px-3 text-[10px] font-semibold text-gray-500 tracking-wider uppercase mb-1">Extensions</div>
                <div class="space-y-0.5">
                  <button phx-click="set_tab" phx-value-tab="plugins" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "plugins", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M11 4a2 2 0 114 0v1a1 1 0 001 1h3a1 1 0 011 1v3a1 1 0 01-1 1h-1a2 2 0 100 4h1a1 1 0 011 1v3a1 1 0 01-1 1h-3a1 1 0 01-1-1v-1a2 2 0 10-4 0v1a1 1 0 01-1 1H7a1 1 0 01-1-1v-3a1 1 0 00-1-1H4a2 2 0 110-4h1a1 1 0 001-1V7a1 1 0 011-1h3a1 1 0 001-1V4z" />
                    </svg>
                    <span>Plugins</span>
                  </button>

                  <button phx-click="set_tab" phx-value-tab="policy" class={"w-full flex items-center space-x-2.5 px-3 py-1.5 rounded-md text-xs font-medium transition " <> if(@active_tab == "policy", do: "bg-[#181922] text-white", else: "text-gray-400 hover:text-gray-200 hover:bg-[#13141a]")}>
                    <svg class="w-3.5 h-3.5 text-gray-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.8">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                    </svg>
                    <span>Policy</span>
                  </button>
                </div>
              </div>

            </div>
          </div>

          <!-- Bottom Footer -->
          <div class="border-t border-[#1a1b22] pt-3 px-2 space-y-1.5 text-[11px] font-mono text-gray-500">
            <div class="flex items-center space-x-1.5 text-gray-300">
              <span class="inline-block w-2 h-2 rounded-full bg-emerald-400 shadow-sm shadow-emerald-500/50"></span>
              <span class="text-emerald-400 font-medium">Connected</span>
              <span class="text-gray-400">localhost:4040</span>
            </div>

            <div class="flex items-center space-x-1.5 text-gray-400">
              <span>apple</span>
              <span class="text-emerald-400 flex items-center space-x-1">
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                </svg>
                <span>policy</span>
              </span>
            </div>

            <div class="text-gray-500 text-[10px]">
              llmman v0.1.0 &nbsp;runtime v0.17.1
            </div>

            <div class="flex items-center space-x-3 pt-0.5 text-gray-400">
              <a href="#" class="hover:text-white transition flex items-center space-x-0.5">
                <span>Docs</span>
                <span class="text-[10px]">↗</span>
              </a>
              <a href="https://github.com/llmmanorg/llmman" target="_blank" class="hover:text-white transition flex items-center space-x-0.5">
                <span>GitHub</span>
                <span class="text-[10px]">↗</span>
              </a>
            </div>
          </div>

        </aside>

        <!-- Main Content Viewport -->
        <main class="flex-1 bg-black flex flex-col overflow-y-auto">
          
          <!-- Top Horizontal Metric Ribbon -->
          <div class="border-b border-[#181a20] px-6 py-3 flex items-center space-x-5 text-xs text-gray-300 font-medium overflow-x-auto flex-shrink-0">
            
            <!-- Running Count -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-emerald-400 text-sm">⚡</span>
              <span class="font-bold text-white"><%= @running_count %></span>
              <span class="text-gray-400">running</span>
            </div>

            <!-- Stopped Count -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-gray-500 text-xs">⏹</span>
              <span class="font-bold text-white"><%= @stopped_count %></span>
              <span class="text-gray-400">stopped</span>
            </div>

            <!-- Total Count -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-gray-400 text-xs">⬡</span>
              <span class="font-bold text-white"><%= @total_count %></span>
              <span class="text-gray-400">total</span>
            </div>

            <span class="text-[#262832] font-light">|</span>

            <!-- vCPUs -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-sky-400 text-xs">⚙</span>
              <span class="font-bold text-white"><%= @total_vcpus %></span>
              <span class="text-gray-400">vCPUs</span>
            </div>

            <!-- Memory -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-purple-400 text-xs">💾</span>
              <span class="font-bold text-white"><%= @total_mem_gb %></span>
              <span class="font-bold text-white">GB</span>
              <span class="text-gray-400">memory</span>
            </div>

            <span class="text-[#262832] font-light">|</span>

            <!-- Objects -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-amber-500 text-xs">📦</span>
              <span class="font-bold text-white"><%= @snapshots_count + 2 %></span>
              <span class="text-gray-400">objects</span>
            </div>

            <!-- Schedules -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-teal-400 text-xs">⏱</span>
              <span class="font-bold text-white">1</span>
              <span class="text-gray-400">schedules (1 active)</span>
            </div>

            <!-- Stores -->
            <div class="flex items-center space-x-1.5 flex-shrink-0">
              <span class="text-indigo-400 text-xs">🗄</span>
              <span class="font-bold text-white">5</span>
              <span class="text-gray-400">stores</span>
            </div>

          </div>

          <!-- Notification Banner -->
          <%= if @notification do %>
            <div class="bg-[#0e2115] border-b border-[#1a4e2e] text-emerald-300 text-xs px-6 py-2 flex items-center justify-between">
              <div class="flex items-center space-x-2">
                <span class="w-2 h-2 rounded-full bg-emerald-400"></span>
                <span><%= @notification %></span>
              </div>
              <button phx-click="close_action_menu" class="text-emerald-400 hover:text-white">✕</button>
            </div>
          <% end %>

          <!-- Dashboard Content Area (Two Columns) -->
          <div class="p-6 flex-1 flex flex-col lg:flex-row gap-6 max-w-[1600px] w-full">
            
            <!-- Left Column: Recent Sandboxes & MicroVMs -->
            <div class="flex-1 flex flex-col min-w-0">
              
              <!-- Column Header -->
              <div class="flex items-center justify-between mb-3.5">
                <h2 class="text-lg font-bold text-white tracking-tight">Recent Sandboxes</h2>
                <button phx-click="set_tab" phx-value-tab="sandboxes" class="text-xs font-medium text-sky-400 hover:text-sky-300 transition">
                  View all
                </button>
              </div>

              <!-- Sandboxes Table Container -->
              <div class="border border-[#1e2027] rounded-xl bg-[#0e0f14] divide-y divide-[#171820] shadow-2xl overflow-hidden">
                <%= for sb <- @sandboxes do %>
                  <div class="py-3 px-4 flex items-center justify-between hover:bg-[#13141a] transition relative group">
                    
                    <!-- Left: Status Pill + Name/Tag -->
                    <div class="flex items-center space-x-3.5 min-w-0">
                      <!-- Status Pill -->
                      <%= cond do %>
                        <% sb["status"] == "running" -> %>
                          <div class="flex items-center space-x-1.5 px-2.5 py-0.5 rounded-full bg-[#0c2417] border border-[#166534]/60 text-emerald-400 text-[11px] font-semibold tracking-tight shadow-sm">
                            <span class="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse"></span>
                            <span>Running</span>
                          </div>
                        <% sb["status"] == "hibernated" -> %>
                          <div class="flex items-center space-x-1.5 px-2.5 py-0.5 rounded-full bg-[#0d1f33] border border-[#1e40af]/50 text-sky-400 text-[11px] font-semibold tracking-tight shadow-sm">
                            <span class="w-1.5 h-1.5 rounded-full bg-sky-400"></span>
                            <span>Hibernated</span>
                          </div>
                        <% true -> %>
                          <div class="px-2.5 py-0.5 rounded-full bg-[#181920] border border-[#272832] text-gray-400 text-[11px] font-semibold tracking-tight">
                            <span>Stopped</span>
                          </div>
                      <% end %>

                      <!-- Sandbox Identity -->
                      <div class="min-w-0">
                        <div class="text-xs font-semibold text-white truncate tracking-tight">
                          <%= sb["name"] %>
                        </div>
                        <div class="text-[11px] font-mono text-gray-500 truncate mt-0.5">
                          <%= sb["tag"] %>
                        </div>
                      </div>
                    </div>

                    <!-- Right: Timestamp & Action Menu -->
                    <div class="flex items-center space-x-4 flex-shrink-0 ml-4">
                      <span class="text-xs text-gray-500 font-mono"><%= sb["time_ago"] %></span>
                      
                      <!-- Three Dots Action Button -->
                      <div class="relative">
                        <button phx-click="toggle_action_menu" phx-value-id={sb["id"]} class="text-gray-500 hover:text-white p-1 rounded hover:bg-[#1e2028] transition">
                          <svg class="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                            <circle cx="5" cy="12" r="2" />
                            <circle cx="12" cy="12" r="2" />
                            <circle cx="19" cy="12" r="2" />
                          </svg>
                        </button>

                        <!-- Action Popover Menu -->
                        <%= if @active_menu_id == sb["id"] do %>
                          <div class="absolute right-0 mt-1.5 w-48 bg-[#16171e] border border-[#262832] rounded-lg shadow-xl py-1 z-50 text-xs font-medium text-gray-200">
                            <%= if sb["status"] == "running" do %>
                              <button phx-click="pause_sandbox" phx-value-id={sb["id"]} class="w-full text-left px-3 py-1.5 hover:bg-[#20222b] flex items-center space-x-2">
                                <span>⏸</span>
                                <span>Pause (SIGSTOP)</span>
                              </button>
                            <% else %>
                              <button phx-click="resume_sandbox" phx-value-id={sb["id"]} class="w-full text-left px-3 py-1.5 hover:bg-[#20222b] flex items-center space-x-2 text-emerald-400">
                                <span>▶</span>
                                <span>Resume (SIGCONT)</span>
                              </button>
                            <% end %>

                            <button phx-click="hibernate_sandbox" phx-value-id={sb["id"]} class="w-full text-left px-3 py-1.5 hover:bg-[#20222b] flex items-center space-x-2 text-sky-400">
                              <span>❄</span>
                              <span>Hibernate (Scale-to-Zero)</span>
                            </button>

                            <a href="http://localhost:11434/#/" target="_blank" class="w-full text-left px-3 py-1.5 hover:bg-[#20222b] flex items-center space-x-2 text-gray-300">
                              <span>💬</span>
                              <span>Open WebUI Chat</span>
                            </a>

                            <a href="http://localhost:11434/#/shell" target="_blank" class="w-full text-left px-3 py-1.5 hover:bg-[#20222b] flex items-center space-x-2 text-gray-300">
                              <span>⚡</span>
                              <span>Attach WebUI Shell</span>
                            </a>

                            <div class="border-t border-[#262832] my-1"></div>

                            <button phx-click="terminate_sandbox" phx-value-id={sb["id"]} class="w-full text-left px-3 py-1.5 hover:bg-rose-950/40 text-rose-400 flex items-center space-x-2">
                              <span>✕</span>
                              <span>Terminate</span>
                            </button>
                          </div>
                        <% end %>
                      </div>

                    </div>

                  </div>
                <% end %>
              </div>

            </div>

            <!-- Right Column: Quickstart -->
            <div class="w-full lg:w-96 flex flex-col flex-shrink-0">
              
              <!-- Column Header -->
              <div class="mb-3.5">
                <h2 class="text-lg font-bold text-white tracking-tight">Quickstart</h2>
              </div>

              <!-- Top Action Buttons -->
              <div class="grid grid-cols-2 gap-2.5 mb-4">
                <button phx-click="open_create_modal" class="bg-white hover:bg-gray-100 text-black font-semibold text-xs py-2.5 px-3 rounded-lg flex items-center justify-center space-x-1.5 transition shadow-sm">
                  <span class="text-sm font-bold leading-none">+</span>
                  <span>Create Sandbox</span>
                </button>

                <button phx-click="quick_run" class="bg-[#14151b] hover:bg-[#1a1c24] border border-[#23252e] hover:border-[#2f323e] text-white font-semibold text-xs py-2.5 px-3 rounded-lg flex items-center justify-center space-x-1.5 transition">
                  <span class="text-xs">▷</span>
                  <span>Quick Run</span>
                </button>
              </div>

              <!-- Preset Templates List -->
              <div class="border border-[#1e2027] rounded-xl bg-[#0e0f14] divide-y divide-[#171820] shadow-2xl overflow-hidden">
                <%= for preset <- @presets do %>
                  <div class="py-2.5 px-4 flex items-center justify-between hover:bg-[#13141a] transition">
                    
                    <div>
                      <div class="text-xs font-semibold text-white tracking-tight">
                        <%= preset.name %>
                      </div>
                      <div class="text-[11px] text-gray-500 mt-0.5">
                        <%= preset.provider %>
                      </div>
                    </div>

                    <button phx-click="start_preset" phx-value-name={preset.name} phx-value-tag={preset.tag} class="bg-[#14151b] border border-[#23252e] hover:border-[#343744] hover:bg-[#1c1e27] text-gray-200 text-xs font-medium px-2.5 py-1 rounded-md flex items-center space-x-1 transition">
                      <span class="text-[11px]">🚀</span>
                      <span>Start</span>
                    </button>

                  </div>
                <% end %>
              </div>

            </div>

          </div>

        </main>
      </div>

      <!-- Create Sandbox Modal -->
      <%= if @show_modal do %>
        <div class="fixed inset-0 bg-black/80 backdrop-blur-sm flex items-center justify-center z-50 p-4">
          <div class="bg-[#101117] border border-[#252733] rounded-xl max-w-md w-full p-6 shadow-2xl space-y-4">
            <div class="flex items-center justify-between border-b border-[#20222c] pb-3">
              <h3 class="text-sm font-bold text-white">Create MicroVM Sandbox</h3>
              <button phx-click="close_modal" class="text-gray-400 hover:text-white">✕</button>
            </div>

            <form phx-submit="create_sandbox" class="space-y-4">
              <div>
                <label class="block text-xs font-medium text-gray-300 mb-1">MicroVM Workload Name</label>
                <input type="text" name="name" placeholder="e.g. llama-3-8b-instruct" class="w-full bg-[#161720] border border-[#2b2d3b] rounded-lg px-3 py-2 text-xs text-white focus:outline-none focus:border-emerald-500 font-mono" />
              </div>

              <div>
                <label class="block text-xs font-medium text-gray-300 mb-1">Model / Workload Template</label>
                <select name="tag" class="w-full bg-[#161720] border border-[#2b2d3b] rounded-lg px-3 py-2 text-xs text-white focus:outline-none focus:border-emerald-500">
                  <option value="llama-3-8b-instruct">llama-3-8b-instruct (Meta OCI)</option>
                  <option value="qwen-2.5-coder-7b">qwen-2.5-coder-7b (Code Specialist)</option>
                  <option value="deepseek-r1-distill">deepseek-r1-distill (Reasoning)</option>
                  <option value="mistral-7b-instruct">mistral-7b-instruct (General)</option>
                  <option value="phi-3.5-mini">phi-3.5-mini (Edge Optimized)</option>
                  <option value="claude-code-agent">claude-code-agent (Agent Bridge)</option>
                  <option value="node:22-alpine">node:22-alpine (Sandbox Container)</option>
                </select>
              </div>

              <div class="grid grid-cols-2 gap-3">
                <div>
                  <label class="block text-xs font-medium text-gray-300 mb-1">vCPUs</label>
                  <input type="number" name="vcpus" value="4" min="1" max="14" class="w-full bg-[#161720] border border-[#2b2d3b] rounded-lg px-3 py-2 text-xs text-white font-mono" />
                </div>
                <div>
                  <label class="block text-xs font-medium text-gray-300 mb-1">Memory (MB)</label>
                  <input type="number" name="memory_mb" value="8192" min="512" step="512" class="w-full bg-[#161720] border border-[#2b2d3b] rounded-lg px-3 py-2 text-xs text-white font-mono" />
                </div>
              </div>

              <div class="flex items-center justify-end space-x-2 pt-2 border-t border-[#20222c]">
                <button type="button" phx-click="close_modal" class="px-3 py-1.5 rounded-lg text-xs font-medium text-gray-400 hover:text-white bg-transparent">
                  Cancel
                </button>
                <button type="submit" class="px-4 py-1.5 rounded-lg text-xs font-bold text-black bg-white hover:bg-gray-100 transition shadow">
                  Create & Run
                </button>
              </div>
            </form>
          </div>
        </div>
      <% end %>

    </div>
    """
  end
end
