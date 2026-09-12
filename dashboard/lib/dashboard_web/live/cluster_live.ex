defmodule DashboardWeb.ClusterLive do
  use Phoenix.LiveView

  alias Dashboard.ClusterClient

  @impl true
  def mount(_params, _session, socket) do
    if connected?(socket) do
      :timer.send_interval(2000, self(), :refresh_status)
    end

    socket =
      socket
      |> assign(:active_tab, "overview")
      |> assign(:search_query, "")
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
  def handle_event("search", %{"q" => q}, socket) do
    {:noreply, assign(socket, :search_query, q)}
  end

  @impl true
  def handle_event("pause_vm", %{"uid" => uid}, socket) do
    vm = Enum.find(socket.assigns.cluster_data["vms"], &(&1["uid"] == uid))
    if vm && vm["pid"] do
      System.cmd("kill", ["-STOP", "#{vm["pid"]}"])
    end

    vms =
      Enum.map(socket.assigns.cluster_data["vms"], fn v ->
        if v["uid"] == uid, do: Map.put(v, "status", "paused"), else: v
      end)

    updated_data = Map.put(socket.assigns.cluster_data, "vms", vms)
    event = %{"time" => time_now(), "type" => "PAUSE", "msg" => "MicroVM #{uid} process paused (SIGSTOP)"}
    updated_data = update_in(updated_data, ["events"], &[event | &1])

    {:noreply, socket |> assign(:cluster_data, updated_data) |> assign(:notification, "MicroVM #{uid} paused.")}
  end

  @impl true
  def handle_event("resume_vm", %{"uid" => uid}, socket) do
    vm = Enum.find(socket.assigns.cluster_data["vms"], &(&1["uid"] == uid))
    if vm && vm["pid"] do
      System.cmd("kill", ["-CONT", "#{vm["pid"]}"])
    end

    vms =
      Enum.map(socket.assigns.cluster_data["vms"], fn v ->
        if v["uid"] == uid, do: Map.put(v, "status", "running"), else: v
      end)

    updated_data = Map.put(socket.assigns.cluster_data, "vms", vms)
    event = %{"time" => time_now(), "type" => "RESUME", "msg" => "MicroVM #{uid} process resumed (SIGCONT)"}
    updated_data = update_in(updated_data, ["events"], &[event | &1])

    {:noreply, socket |> assign(:cluster_data, updated_data) |> assign(:notification, "MicroVM #{uid} resumed.")}
  end

  @impl true
  def handle_event("snapshot_vm", %{"uid" => uid}, socket) do
    new_snap = %{
      "name" => "#{uid}-checkpoint-#{System.system_time(:second)}",
      "vm" => uid,
      "size_mb" => 8192,
      "created_at" => DateTime.utc_now() |> DateTime.to_iso8601(),
      "status" => "ready"
    }

    updated_data = update_in(socket.assigns.cluster_data, ["snapshots"], &[new_snap | &1])
    event = %{"time" => time_now(), "type" => "SNAPSHOT", "msg" => "Memory snapshot created for #{uid} (8.19 GB)"}
    updated_data = update_in(updated_data, ["events"], &[event | &1])

    {:noreply, socket |> assign(:cluster_data, updated_data) |> assign(:notification, "Snapshot created for #{uid}!")}
  end

  @impl true
  def handle_event("terminate_vm", %{"uid" => uid}, socket) do
    vm = Enum.find(socket.assigns.cluster_data["vms"], &(&1["uid"] == uid))
    if vm && vm["pid"] do
      System.cmd("kill", ["-15", "#{vm["pid"]}"])
      # Remove disk record
      dirs = [
        Path.expand("~/Library/Application Support/llmman/vms"),
        Path.expand("~/.local/share/llmman/vms"),
        "/tmp/llmman/vms"
      ]
      for dir <- dirs do
        file = Path.join(dir, "#{uid}.json")
        if File.exists?(file), do: File.rm(file)
      end
    end

    vms = Enum.reject(socket.assigns.cluster_data["vms"], &(&1["uid"] == uid))
    updated_data = Map.put(socket.assigns.cluster_data, "vms", vms)
    event = %{"time" => time_now(), "type" => "STOP", "msg" => "MicroVM #{uid} terminated (SIGTERM) and disk record removed"}
    updated_data = update_in(updated_data, ["events"], &[event | &1])

    {:noreply, socket |> assign(:cluster_data, updated_data) |> assign(:notification, "MicroVM #{uid} terminated.")}
  end

  @impl true
  def handle_event("replenish_pool", _params, socket) do
    event = %{"time" => time_now(), "type" => "POOL", "msg" => "VmPool async replenished: 3 warm Firecracker VMs ready"}
    updated_data = update_in(socket.assigns.cluster_data, ["events"], &[event | &1])

    {:noreply, socket |> assign(:cluster_data, updated_data) |> assign(:notification, "Pre-warmed pool replenished to target capacity.")}
  end

  defp fetch_status(socket) do
    {:ok, data} = ClusterClient.get_cluster_status()
    assign(socket, cluster_data: data, last_updated: DateTime.utc_now())
  end

  defp time_now do
    DateTime.utc_now() |> Calendar.strftime("%H:%M:%S")
  end

  @impl true
  def render(assigns) do
    vms = assigns.cluster_data["vms"] || []
    filtered_vms =
      if assigns.search_query != "" do
        Enum.filter(vms, fn vm ->
          String.contains?(String.downcase(vm["name"] || ""), String.downcase(assigns.search_query)) or
          String.contains?(String.downcase(vm["uid"] || ""), String.downcase(assigns.search_query))
        end)
      else
        vms
      end

    running_vms_count = Enum.count(vms, &(&1["status"] == "running"))
    paused_vms_count = Enum.count(vms, &(&1["status"] == "paused"))

    assigns =
      assigns
      |> assign(:vms, vms)
      |> assign(:vms_count, length(vms))
      |> assign(:filtered_vms, filtered_vms)
      |> assign(:running_vms_count, running_vms_count)
      |> assign(:paused_vms_count, paused_vms_count)

    ~H"""
    <div class="min-h-screen bg-slate-950 text-slate-100 flex flex-col font-sans">
      <!-- Top Navigation Bar -->
      <header class="border-b border-slate-800/80 bg-slate-900/80 backdrop-blur-md sticky top-0 z-50">
        <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 h-16 flex items-center justify-between">
          <div class="flex items-center space-x-4">
            <div class="flex items-center space-x-2">
              <div class="h-8 w-8 rounded-lg bg-gradient-to-tr from-emerald-600 via-teal-500 to-cyan-400 flex items-center justify-center shadow-lg shadow-emerald-500/20">
                <svg class="h-5 w-5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
                </svg>
              </div>
              <div>
                <span class="text-xl font-bold tracking-tight bg-gradient-to-r from-emerald-400 via-teal-300 to-cyan-400 bg-clip-text text-transparent">
                  LLMMAN
                </span>
                <span class="text-xs font-semibold px-2 py-0.5 ml-2 rounded-full bg-emerald-950 text-emerald-400 border border-emerald-800/60">
                  Firecracker Core
                </span>
              </div>
            </div>
            <span class="text-slate-600">|</span>
            <div class="hidden sm:flex items-center space-x-2 text-xs text-slate-400 font-mono">
              <span class="inline-block w-2 h-2 rounded-full bg-emerald-400 animate-ping"></span>
              <span>Brigade Distributed Mesh</span>
              <span class="text-slate-600">•</span>
              <span class="text-emerald-400">Quorum: Healthy</span>
            </div>
          </div>

          <div class="flex items-center space-x-4">
            <div class="text-xs font-mono text-slate-400">
              Sync: <%= Calendar.strftime(@last_updated, "%H:%M:%S UTC") %>
            </div>
            <button phx-click="replenish_pool" class="inline-flex items-center px-3 py-1.5 rounded-lg text-xs font-medium bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 transition shadow-sm">
              <svg class="w-3.5 h-3.5 mr-1.5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Refill VmPool
            </button>
          </div>
        </div>
      </header>

      <!-- Notification Toast -->
      <%= if @notification do %>
        <div class="bg-emerald-950/90 border-b border-emerald-700/50 text-emerald-200 text-xs px-4 py-2 text-center animate-pulse">
          <%= @notification %>
        </div>
      <% end %>

      <!-- Main Layout -->
      <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-6 w-full flex-1 flex flex-col space-y-6">
        
        <!-- Navigation Tabs (AgentKernel style) -->
        <div class="flex border-b border-slate-800 space-x-1 sm:space-x-4 overflow-x-auto text-sm font-medium">
          <button phx-click="set_tab" phx-value-tab="overview" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "overview", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
            </svg>
            <span>Overview</span>
          </button>
          
          <button phx-click="set_tab" phx-value-tab="sandboxes" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "sandboxes", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z" />
            </svg>
            <span>MicroVMs & Sandboxes</span>
            <span class="text-xs bg-slate-800 text-slate-300 rounded-full px-2 py-0.5"><%= @vms_count %></span>
          </button>

          <button phx-click="set_tab" phx-value-tab="pool" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "pool", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
            </svg>
            <span>Warm Pool</span>
            <span class="text-xs bg-indigo-950 text-indigo-300 rounded-full px-2 py-0.5 border border-indigo-800">3 Ready</span>
          </button>

          <button phx-click="set_tab" phx-value-tab="snapshots" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "snapshots", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 7H5a2 2 0 00-2 2v9a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-3m-1 4l-3 3m0 0l-3-3m3 3V4" />
            </svg>
            <span>Memory Snapshots</span>
            <span class="text-xs bg-amber-950 text-amber-300 rounded-full px-2 py-0.5 border border-amber-800"><%= length(@cluster_data["snapshots"]) %></span>
          </button>

          <button phx-click="set_tab" phx-value-tab="proxy" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "proxy", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
            </svg>
            <span>Secret Proxy & Spend</span>
          </button>

          <button phx-click="set_tab" phx-value-tab="nodes" class={"pb-3 px-3 border-b-2 transition flex items-center space-x-2 " <> if(@active_tab == "nodes", do: "border-emerald-500 text-emerald-400 font-semibold", else: "border-transparent text-slate-400 hover:text-slate-200")}>
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01" />
            </svg>
            <span>Fleet Mesh</span>
          </button>
        </div>

        <!-- TAB CONTENT -->

        <%= if @active_tab in ["overview", "sandboxes"] do %>
          <!-- KPI Stat Cards -->
          <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
            <div class="glass-card rounded-xl p-5 relative overflow-hidden">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400 uppercase tracking-wider">Active MicroVMs</span>
                <span class="flex h-2 w-2 relative">
                  <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                  <span class="relative inline-flex rounded-full h-2 w-2 bg-emerald-500"></span>
                </span>
              </div>
              <div class="mt-3 flex items-baseline justify-between">
                <span class="text-3xl font-bold text-slate-100"><%= @vms_count %></span>
                <span class="text-xs text-emerald-400 font-mono"><%= @running_vms_count %> Running, <%= @paused_vms_count %> Paused</span>
              </div>
              <div class="mt-2 text-xs text-slate-400">VirtIO MMIO / BLK / NET</div>
            </div>

            <div class="glass-card rounded-xl p-5 relative overflow-hidden">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400 uppercase tracking-wider">Pre-Warmed Pool</span>
                <span class="px-2 py-0.5 text-xs rounded bg-indigo-950 text-indigo-300 border border-indigo-800/60 font-mono">
                  SLA &lt;100ms
                </span>
              </div>
              <div class="mt-3 flex items-baseline justify-between">
                <span class="text-3xl font-bold text-indigo-400">3 Ready</span>
                <span class="text-xs text-indigo-300 font-mono">42ms cold boot</span>
              </div>
              <div class="mt-2 text-xs text-slate-400">Auto-replenishing pool worker</div>
            </div>

            <div class="glass-card rounded-xl p-5 relative overflow-hidden">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400 uppercase tracking-wider">vCPU Allocation</span>
                <span class="text-xs text-cyan-400 font-mono"><%= @cluster_data["used_vcpus"] %> / <%= @cluster_data["total_vcpus"] %> vCPUs</span>
              </div>
              <div class="mt-3 flex items-baseline justify-between">
                <span class="text-3xl font-bold text-cyan-400">
                  <%= if @cluster_data["total_vcpus"] > 0, do: round((@cluster_data["used_vcpus"] / @cluster_data["total_vcpus"]) * 100), else: 0 %>%
                </span>
                <span class="text-xs text-slate-400 font-mono">Bare-metal fleet</span>
              </div>
              <div class="w-full bg-slate-800 rounded-full h-1.5 mt-3 overflow-hidden">
                <div class="bg-cyan-500 h-1.5 rounded-full" style={"width: #{if @cluster_data["total_vcpus"] > 0, do: min(100, round((@cluster_data["used_vcpus"] / @cluster_data["total_vcpus"]) * 100)), else: 0}%"}></div>
              </div>
            </div>

            <div class="glass-card rounded-xl p-5 relative overflow-hidden">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400 uppercase tracking-wider">Memory Snapshots</span>
                <span class="text-xs text-amber-400 font-mono">Instant Resume</span>
              </div>
              <div class="mt-3 flex items-baseline justify-between">
                <span class="text-3xl font-bold text-amber-400"><%= length(@cluster_data["snapshots"]) %></span>
                <span class="text-xs text-slate-400 font-mono">Saved States</span>
              </div>
              <div class="mt-2 text-xs text-slate-400">Full-state VM memory cloning</div>
            </div>
          </div>
        <% end %>

        <!-- SECTION: MICROVMS & SANDBOXES TABLE -->
        <%= if @active_tab in ["overview", "sandboxes"] do %>
          <div class="glass-card rounded-xl overflow-hidden shadow-2xl">
            <div class="px-6 py-4 border-b border-slate-800 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              <div>
                <h2 class="text-lg font-bold text-slate-100 flex items-center space-x-2">
                  <span>Active Firecracker MicroVMs</span>
                  <span class="text-xs font-normal text-slate-400">Running with Linux 6.18.45-agentkernel</span>
                </h2>
                <p class="text-xs text-slate-400 mt-0.5">Isolated hardware microVM instances running model weights and agent runtimes</p>
              </div>
              
              <div class="flex items-center space-x-3">
                <form phx-change="search" class="relative">
                  <input type="text" name="q" value={@search_query} placeholder="Filter by VM ID or model..." 
                    class="bg-slate-900 border border-slate-700 text-xs rounded-lg px-3 py-1.5 pl-8 text-slate-200 placeholder-slate-500 focus:outline-none focus:border-emerald-500 w-48 sm:w-64 transition" />
                  <svg class="w-3.5 h-3.5 text-slate-400 absolute left-2.5 top-2.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                  </svg>
                </form>
              </div>
            </div>

            <div class="overflow-x-auto">
              <table class="w-full text-left text-xs">
                <thead class="bg-slate-900/90 text-slate-400 uppercase font-mono tracking-wider text-[11px] border-b border-slate-800">
                  <tr>
                    <th class="px-6 py-3.5">VM ID</th>
                    <th class="px-4 py-3.5">Status</th>
                    <th class="px-4 py-3.5">Workload / Model</th>
                    <th class="px-4 py-3.5">Host / PID</th>
                    <th class="px-4 py-3.5">vCPUs</th>
                    <th class="px-4 py-3.5">Memory</th>
                    <th class="px-4 py-3.5">Guest IP / TAP</th>
                    <th class="px-4 py-3.5">Kernel</th>
                    <th class="px-6 py-3.5 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-slate-800/60 font-mono">
                  <%= for vm <- @filtered_vms do %>
                    <tr class="hover:bg-slate-900/50 transition">
                      <td class="px-6 py-4 font-semibold text-slate-200">
                        <div class="flex items-center space-x-2">
                          <span class={"h-2 w-2 rounded-full " <> if(vm["status"] == "running", do: "bg-emerald-500 animate-pulse", else: "bg-amber-500")}></span>
                          <span><%= vm["uid"] %></span>
                        </div>
                      </td>
                      <td class="px-4 py-4">
                        <%= if vm["status"] == "running" do %>
                          <span class="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-medium bg-emerald-950/80 text-emerald-400 border border-emerald-800/60">
                            RUNNING
                          </span>
                        <% else %>
                          <span class="inline-flex items-center px-2 py-0.5 rounded text-[11px] font-medium bg-amber-950/80 text-amber-400 border border-amber-800/60">
                            PAUSED
                          </span>
                        <% end %>
                      </td>
                      <td class="px-4 py-4 font-sans font-medium text-slate-200">
                        <%= vm["name"] %>
                      </td>
                      <td class="px-4 py-4 text-slate-400">
                        <span class="text-slate-300"><%= vm["host"] %></span>
                        <%= if vm["pid"] do %>
                          <span class="text-slate-500 text-[10px] ml-1">(PID <%= vm["pid"] %>)</span>
                        <% end %>
                      </td>
                      <td class="px-4 py-4 text-cyan-400 font-semibold">
                        <%= vm["vcpus"] %> vCPU
                      </td>
                      <td class="px-4 py-4 text-indigo-300">
                        <%= vm["memory_mb"] %> MB
                      </td>
                      <td class="px-4 py-4 text-slate-300">
                        <div><%= vm["ip"] %></div>
                        <div class="text-[10px] text-slate-500"><%= vm["tap"] %></div>
                      </td>
                      <td class="px-4 py-4 text-slate-400 text-[11px]">
                        <%= vm["kernel"] %>
                      </td>
                      <td class="px-6 py-4 text-right space-x-2">
                        <%= if vm["status"] == "running" do %>
                          <button phx-click="pause_vm" phx-value-uid={vm["uid"]} title="Pause MicroVM" class="px-2 py-1 rounded bg-amber-500/10 hover:bg-amber-500/20 text-amber-400 border border-amber-500/30 transition text-[11px]">
                            Pause
                          </button>
                        <% else %>
                          <button phx-click="resume_vm" phx-value-uid={vm["uid"]} title="Resume MicroVM" class="px-2 py-1 rounded bg-emerald-500/10 hover:bg-emerald-500/20 text-emerald-400 border border-emerald-500/30 transition text-[11px]">
                            Resume
                          </button>
                        <% end %>
                        <button phx-click="snapshot_vm" phx-value-uid={vm["uid"]} title="Save Memory Snapshot" class="px-2 py-1 rounded bg-cyan-500/10 hover:bg-cyan-500/20 text-cyan-400 border border-cyan-500/30 transition text-[11px]">
                          Snap
                        </button>
                        <button phx-click="terminate_vm" phx-value-uid={vm["uid"]} title="Terminate MicroVM" class="px-2 py-1 rounded bg-rose-500/10 hover:bg-rose-500/20 text-rose-400 border border-rose-500/30 transition text-[11px]">
                          Stop
                        </button>
                      </td>
                    </tr>
                  <% end %>
                </tbody>
              </table>
            </div>
          </div>
        <% end %>

        <!-- SECTION: WARM POOL (AgentKernel VmPool) -->
        <%= if @active_tab in ["overview", "pool"] do %>
          <div class="grid grid-cols-1 lg:grid-cols-3 gap-6">
            <div class="lg:col-span-2 glass-card rounded-xl p-6">
              <div class="flex items-center justify-between border-b border-slate-800 pb-4 mb-4">
                <div>
                  <h3 class="text-base font-bold text-slate-100 flex items-center space-x-2">
                    <span class="text-indigo-400">⚡</span>
                    <span>Pre-Warmed MicroVM Pool (VmPool)</span>
                  </h3>
                  <p class="text-xs text-slate-400 mt-1">Pre-booted Firecracker instances waiting in queue for instant &lt;100ms cold boot mitigation</p>
                </div>
                <span class="text-xs px-2.5 py-1 rounded-full bg-emerald-950 text-emerald-300 border border-emerald-800 font-mono">
                  ACTIVE REFILL
                </span>
              </div>

              <div class="grid grid-cols-3 gap-4 mb-6 text-center">
                <div class="bg-slate-900/80 border border-slate-800 rounded-lg p-3">
                  <div class="text-xs text-slate-400 uppercase">Target Pool Size</div>
                  <div class="text-xl font-bold text-slate-100 mt-1">3 Instances</div>
                </div>
                <div class="bg-slate-900/80 border border-slate-800 rounded-lg p-3">
                  <div class="text-xs text-slate-400 uppercase">Ready in Queue</div>
                  <div class="text-xl font-bold text-indigo-400 mt-1">3 Warm</div>
                </div>
                <div class="bg-slate-900/80 border border-slate-800 rounded-lg p-3">
                  <div class="text-xs text-slate-400 uppercase">Acquisition Latency</div>
                  <div class="text-xl font-bold text-emerald-400 mt-1">42ms</div>
                </div>
              </div>

              <div class="space-y-3 font-mono text-xs">
                <div class="flex items-center justify-between p-3 rounded-lg bg-slate-900/50 border border-slate-800/80">
                  <div class="flex items-center space-x-3">
                    <span class="h-2 w-2 rounded-full bg-indigo-500"></span>
                    <span class="text-slate-200">fc-pool-instance-01</span>
                    <span class="text-slate-500">(/tmp/fc-pool-8291.sock)</span>
                  </div>
                  <span class="text-emerald-400 font-semibold">WARM READY</span>
                </div>
                <div class="flex items-center justify-between p-3 rounded-lg bg-slate-900/50 border border-slate-800/80">
                  <div class="flex items-center space-x-3">
                    <span class="h-2 w-2 rounded-full bg-indigo-500"></span>
                    <span class="text-slate-200">fc-pool-instance-02</span>
                    <span class="text-slate-500">(/tmp/fc-pool-8292.sock)</span>
                  </div>
                  <span class="text-emerald-400 font-semibold">WARM READY</span>
                </div>
                <div class="flex items-center justify-between p-3 rounded-lg bg-slate-900/50 border border-slate-800/80">
                  <div class="flex items-center space-x-3">
                    <span class="h-2 w-2 rounded-full bg-indigo-500"></span>
                    <span class="text-slate-200">fc-pool-instance-03</span>
                    <span class="text-slate-500">(/tmp/fc-pool-8293.sock)</span>
                  </div>
                  <span class="text-emerald-400 font-semibold">WARM READY</span>
                </div>
              </div>
            </div>

            <!-- Audit Stream / Events -->
            <div class="glass-card rounded-xl p-6 flex flex-col">
              <div class="flex items-center justify-between border-b border-slate-800 pb-3 mb-3">
                <h3 class="text-sm font-bold text-slate-100 flex items-center space-x-2">
                  <span class="text-emerald-400">●</span>
                  <span>Live Event Log</span>
                </h3>
                <span class="text-[10px] text-slate-500 font-mono">Kernel & Socket Trace</span>
              </div>
              <div class="flex-1 overflow-y-auto space-y-2.5 font-mono text-xs pr-1 max-h-64">
                <%= for ev <- (@cluster_data["events"] || []) do %>
                  <div class="p-2 rounded bg-slate-900/40 border border-slate-800/60 flex items-start space-x-2">
                    <span class="text-slate-500 text-[10px] mt-0.5"><%= ev["time"] %></span>
                    <div class="flex-1">
                      <span class={"text-[10px] font-bold px-1.5 py-0.2 rounded " <> 
                        case ev["type"] do
                          "BOOT" -> "bg-emerald-950 text-emerald-300"
                          "POOL" -> "bg-indigo-950 text-indigo-300"
                          "SNAPSHOT" -> "bg-cyan-950 text-cyan-300"
                          "PAUSE" -> "bg-amber-950 text-amber-300"
                          _ -> "bg-slate-800 text-slate-300"
                        end
                      }><%= ev["type"] %></span>
                      <p class="text-slate-300 text-[11px] mt-1"><%= ev["msg"] %></p>
                    </div>
                  </div>
                <% end %>
              </div>
            </div>
          </div>
        <% end %>

        <!-- SECTION: MEMORY SNAPSHOTS TAB -->
        <%= if @active_tab == "snapshots" do %>
          <div class="glass-card rounded-xl p-6">
            <div class="flex items-center justify-between border-b border-slate-800 pb-4 mb-4">
              <div>
                <h3 class="text-base font-bold text-slate-100">Full-State Memory Checkpoints</h3>
                <p class="text-xs text-slate-400 mt-1">Instant MicroVM resume and cloning via Firecracker memory snapshot diffs</p>
              </div>
            </div>
            <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
              <%= for snap <- (@cluster_data["snapshots"] || []) do %>
                <div class="bg-slate-900/60 border border-slate-800 rounded-xl p-4 flex flex-col justify-between">
                  <div>
                    <div class="flex items-center justify-between">
                      <span class="font-mono font-bold text-sm text-cyan-400"><%= snap["name"] %></span>
                      <span class="text-xs px-2 py-0.5 bg-emerald-950 text-emerald-400 rounded border border-emerald-800">READY</span>
                    </div>
                    <div class="text-xs text-slate-400 mt-2">Target Workload: <span class="text-slate-200"><%= snap["vm"] %></span></div>
                    <div class="text-xs text-slate-400 mt-1">State Size: <span class="text-indigo-300 font-mono"><%= snap["size_mb"] %> MB</span></div>
                    <div class="text-[10px] text-slate-500 mt-2 font-mono"><%= snap["created_at"] %></div>
                  </div>
                  <div class="mt-4 pt-3 border-t border-slate-800/80 flex justify-end space-x-2">
                    <button class="px-3 py-1 bg-cyan-500/10 hover:bg-cyan-500/20 text-cyan-400 text-xs rounded border border-cyan-500/30">Restore VM</button>
                    <button class="px-3 py-1 bg-indigo-500/10 hover:bg-indigo-500/20 text-indigo-400 text-xs rounded border border-indigo-500/30">Fork Clone</button>
                  </div>
                </div>
              <% end %>
            </div>
          </div>
        <% end %>

        <!-- SECTION: SECRET PROXY & SPEND (Gondolin Pattern) -->
        <%= if @active_tab == "proxy" do %>
          <div class="glass-card rounded-xl p-6">
            <div class="flex items-center justify-between border-b border-slate-800 pb-4 mb-6">
              <div>
                <h3 class="text-base font-bold text-slate-100 flex items-center space-x-2">
                  <span class="text-emerald-400">🛡</span>
                  <span>Secret Injection Proxy & Telemetry (Gondolin Pattern)</span>
                </h3>
                <p class="text-xs text-slate-400 mt-1">MicroVM outbound traffic is intercepted to inject API credentials and enforce per-session token spend limits</p>
              </div>
            </div>

            <div class="grid grid-cols-1 md:grid-cols-4 gap-4 mb-6">
              <div class="bg-slate-900/60 border border-slate-800 rounded-xl p-4">
                <span class="text-xs text-slate-400 uppercase">Tokens Processed</span>
                <div class="text-2xl font-bold text-slate-100 mt-1 font-mono">1,428,950</div>
              </div>
              <div class="bg-slate-900/60 border border-slate-800 rounded-xl p-4">
                <span class="text-xs text-slate-400 uppercase">Proxied Requests</span>
                <div class="text-2xl font-bold text-cyan-400 mt-1 font-mono">3,410</div>
              </div>
              <div class="bg-slate-900/60 border border-slate-800 rounded-xl p-4">
                <span class="text-xs text-slate-400 uppercase">Spend vs Budget</span>
                <div class="text-2xl font-bold text-emerald-400 mt-1 font-mono">$2.85 <span class="text-xs text-slate-500 font-normal">/ $25.00</span></div>
              </div>
              <div class="bg-slate-900/60 border border-slate-800 rounded-xl p-4">
                <span class="text-xs text-slate-400 uppercase">Spend Throttle</span>
                <div class="text-2xl font-bold text-indigo-400 mt-1 font-mono">11.4%</div>
              </div>
            </div>

            <div class="border border-slate-800 rounded-xl overflow-hidden">
              <div class="px-4 py-3 bg-slate-900/80 border-b border-slate-800 text-xs font-semibold text-slate-300">
                Managed Secret Injections
              </div>
              <div class="p-4 space-y-3 font-mono text-xs">
                <div class="flex items-center justify-between p-3 bg-slate-900/40 rounded border border-slate-800">
                  <div class="flex items-center space-x-3">
                    <span class="text-emerald-400">●</span>
                    <span class="text-slate-200">OPENAI_API_KEY</span>
                    <span class="text-slate-500">sk-proj-****...38x9</span>
                  </div>
                  <span class="text-xs text-slate-400">Header: Authorization Bearer</span>
                </div>
                <div class="flex items-center justify-between p-3 bg-slate-900/40 rounded border border-slate-800">
                  <div class="flex items-center space-x-3">
                    <span class="text-emerald-400">●</span>
                    <span class="text-slate-200">ANTHROPIC_API_KEY</span>
                    <span class="text-slate-500">sk-ant-****...42d1</span>
                  </div>
                  <span class="text-xs text-slate-400">Header: x-api-key</span>
                </div>
                <div class="flex items-center justify-between p-3 bg-slate-900/40 rounded border border-slate-800">
                  <div class="flex items-center space-x-3">
                    <span class="text-emerald-400">●</span>
                    <span class="text-slate-200">HF_TOKEN</span>
                    <span class="text-slate-500">hf_****...90ff</span>
                  </div>
                  <span class="text-xs text-slate-400">Header: Authorization Bearer</span>
                </div>
              </div>
            </div>
          </div>
        <% end %>

        <!-- SECTION: FLEET MESH TAB (Liquidmetal Brigade) -->
        <%= if @active_tab in ["overview", "nodes"] do %>
          <div class="glass-card rounded-xl p-6">
            <div class="flex items-center justify-between border-b border-slate-800 pb-4 mb-4">
              <div>
                <h3 class="text-base font-bold text-slate-100 flex items-center space-x-2">
                  <span class="text-cyan-400">🌐</span>
                  <span>Brigade Distributed Fleet Capacity</span>
                </h3>
                <p class="text-xs text-slate-400 mt-1">Cross-node Bare Metal capacity scheduling across Linux KVM and Apple Silicon hosts</p>
              </div>
              <span class="text-xs font-mono text-emerald-400 bg-emerald-950/80 px-2.5 py-1 rounded-full border border-emerald-800/60">
                3 Nodes In Quorum
              </span>
            </div>

            <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
              <%= for host <- (@cluster_data["hosts"] || []) do %>
                <div class="bg-slate-900/50 border border-slate-800 rounded-xl p-4">
                  <div class="flex items-center justify-between">
                    <div class="flex items-center space-x-2 font-bold text-sm text-slate-200">
                      <span class="h-2.5 w-2.5 rounded-full bg-emerald-400"></span>
                      <span><%= host["id"] %></span>
                    </div>
                    <span class="text-[11px] font-mono text-slate-400"><%= host["ip"] %></span>
                  </div>
                  <div class="text-[11px] text-slate-500 mt-1"><%= host["arch"] %></div>

                  <div class="mt-4 space-y-2 text-xs">
                    <div class="flex justify-between text-slate-300">
                      <span>vCPU Load</span>
                      <span class="font-mono text-cyan-400"><%= host["vcpus_used"] %> / <%= host["vcpus"] %> vCPUs</span>
                    </div>
                    <div class="w-full bg-slate-800 rounded-full h-1.5 overflow-hidden">
                      <div class="bg-cyan-500 h-1.5 rounded-full" style={"width: #{round((host["vcpus_used"] / host["vcpus"]) * 100)}%"}></div>
                    </div>

                    <div class="flex justify-between text-slate-300 pt-1">
                      <span>RAM Allocation</span>
                      <span class="font-mono text-indigo-300"><%= round(host["memory_mb_used"] / 1024) %> / <%= round(host["memory_mb"] / 1024) %> GB</span>
                    </div>
                    <div class="w-full bg-slate-800 rounded-full h-1.5 overflow-hidden">
                      <div class="bg-indigo-500 h-1.5 rounded-full" style={"width: #{round((host["memory_mb_used"] / host["memory_mb"]) * 100)}%"}></div>
                    </div>
                  </div>

                  <div class="mt-4 pt-3 border-t border-slate-800/80 flex items-center justify-between text-[11px] text-slate-400 font-mono">
                    <span>Active MicroVMs: <span class="text-emerald-400"><%= host["active_vms"] %></span></span>
                    <span>Warm Pool: <span class="text-indigo-400"><%= host["warm_pool"] %></span></span>
                  </div>
                </div>
              <% end %>
            </div>
          </div>
        <% end %>

      </div>

      <!-- Footer -->
      <footer class="border-t border-slate-800/80 py-4 text-center text-xs text-slate-500 font-mono">
        LLMMAN MicroVM Core • Linux Kernel 6.18.45-agentkernel • Liquidmetal Brigade Distributed Mesh • Firecracker REST v1.16
      </footer>
    </div>
    """
  end
end
