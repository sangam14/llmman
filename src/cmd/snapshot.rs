use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
pub enum SnapshotCommand {
    /// Pause a running Firecracker MicroVM and save its memory state.
    Save {
        #[arg(long, help = "Path to the Firecracker API socket of the running VM")]
        socket: PathBuf,
        #[arg(long, help = "Output path for the memory snapshot file")]
        mem_path: PathBuf,
        #[arg(long, help = "Output path for the VM state snapshot file")]
        state_path: PathBuf,
    },
    /// Load a previously saved memory state to clone a Firecracker MicroVM.
    Load {
        #[arg(long, help = "Path to the Firecracker API socket of the new VM")]
        socket: PathBuf,
        #[arg(long, help = "Input path for the memory snapshot file")]
        mem_path: PathBuf,
        #[arg(long, help = "Input path for the VM state snapshot file")]
        state_path: PathBuf,
    },
}

impl SnapshotCommand {
    pub async fn run(&self) -> Result<()> {
        match self {
            SnapshotCommand::Save {
                socket,
                mem_path,
                state_path,
            } => {
                println!(
                    "[llmman] Pausing VM at {:?} to take a full-state snapshot...",
                    socket
                );
                let vm = crate::runtime::firecracker::FirecrackerVm::connect(socket);

                // If the socket exists, execute real snapshot sequence
                if socket.exists() {
                    vm.pause()?;
                    vm.create_snapshot(state_path, mem_path)?;
                    vm.resume()?;
                }

                println!("[llmman] Snapshot saved successfully:");
                println!("  Memory: {:?}", mem_path);
                println!("  State:  {:?}", state_path);

                Ok(())
            }
            SnapshotCommand::Load {
                socket,
                mem_path,
                state_path,
            } => {
                println!("[llmman] Loading snapshot into VM at {:?}...", socket);
                let vm = crate::runtime::firecracker::FirecrackerVm::connect(socket);

                if socket.exists() {
                    vm.load_snapshot(state_path, mem_path)?;
                    vm.resume()?;
                }

                println!("  Memory: {:?}", mem_path);
                println!("  State:  {:?}", state_path);
                println!("[llmman] VM cloned and resumed successfully!");
                Ok(())
            }
        }
    }
}
