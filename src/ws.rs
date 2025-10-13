use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{stream::StreamExt, SinkExt};
use std::time::Duration;
use tracing::info;

use crate::job_tracking::{
    manager::TaskManager,
    task::{SerializableTaskStatus, TaskUpdate},
};

///
/// # Panics
///
/// Panics if the shared task manager mutex is poisoned while serializing
/// the initial snapshot sent to the client.
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    info!("WebSocket connection request received at /ws/status");
    ws.on_upgrade(move |socket| async move {
        info!("WebSocket connection established successfully");
        let task_manager = TaskManager::global();

        let (mut sender, mut receiver) = socket.split();
        let mut rx = task_manager.tx.subscribe();

        // Send fresh snapshot to clients.
        task_manager.broadcast_update();

        let task_update = {
            let task_list = {
                let tasks = task_manager.tasks.lock().unwrap();
                tasks
                    .values()
                    .map(|task| SerializableTaskStatus {
                        id: task.id.clone(),
                        task_type: task.task_type.clone(),
                        title: task.title.clone(),
                        state: task.state.clone(),
                        status: task.status.clone(),
                    })
                    .collect::<Vec<SerializableTaskStatus>>()
            };
            TaskUpdate { tasks: task_list }
        };

        if let Ok(msg) = serde_json::to_string(&task_update) {
            if let Err(e) = sender.send(Message::Text(msg.into())).await {
                info!("Error sending initial message: {:?}", e);
                return;
            }
        }

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = sender.send(Message::Ping(vec![1, 2, 3, 4].into())).await {
                            info!("Ping failed: {:?}", e);
                            break;
                        }
                    }
                    update = rx.recv() => {
                        if let Ok(update) = update {
                            if let Ok(msg) = serde_json::to_string(&update) {
                                if sender.send(Message::Text(msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });

        while let Some(Ok(_)) = receiver.next().await {
            // Keep the connection alive. All updates are broadcast driven.
        }
    })
}
