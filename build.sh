export HF_API_TOKEN=$(grep HF_API_TOKEN .env | cut -d '=' -f2)
cargo tauri build