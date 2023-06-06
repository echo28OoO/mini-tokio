use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use futures::task::{self, ArcWake};
use std::thread;
use crossbeam::channel;
use std::sync::{Arc, Mutex};

fn main() {
    let mut mini_tokio = MiniTokio::new();

    mini_tokio.spawn(async {
        let when = Instant::now() + Duration::from_millis(10);
        let future = Delay { when };

        let out = future.await;
        assert_eq!(out, "done");
    });

    mini_tokio.run();
}

struct MiniTokio {
    scheduled: channel::Receiver<Arc<Task>>,
    sender: channel::Sender<Arc<Task>>,
}

struct Task {
    // `Mutex` 是为了让 `Task` 实现 `Sync` 特征，它能保证同一时间只有一个线程可以访问 `Future`。
    // 事实上 `Mutex` 并没有在 Tokio 中被使用，这里我们只是为了简化： Tokio 的真实代码实在太长了 :D
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
    executor: channel::Sender<Arc<Task>>,
}

impl Task {
    fn schedule(self: &Arc<Self>) {
        self.executor.send(self.clone());
    }
}

impl Task {
    fn poll(self: Arc<Self>) {
        // 基于 Task 实例创建一个 waker, 它使用了之前的 `ArcWake`
        let waker = task::waker(self.clone());
        let mut cx = Context::from_waker(&waker);

        // 没有其他线程在竞争锁时，我们将获取到目标 future
        let mut future = self.future.try_lock().unwrap();

        // 对 future 进行 poll
        let _ = future.as_mut().poll(&mut cx);
    }

    // 使用给定的 future 来生成新的任务
    //
    // 新的任务会被推到 `sender` 中，接着该消息通道的接收端就可以获取该任务，然后执行
    fn spawn<F>(future: F, sender: &channel::Sender<Arc<Task>>)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            executor: sender.clone(),
        });

        let _ = sender.send(task);
    }

}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.schedule();
    }
}

impl MiniTokio {
    // 从消息通道中接收任务，然后通过 poll 来执行
    fn run(&self) {
        while let Ok(task) = self.scheduled.recv() {
            task.poll();
        }
    }

    /// 初始化一个新的 mini-tokio 实例
    fn new() -> MiniTokio {
        let (sender, scheduled) = channel::unbounded();

        MiniTokio { scheduled, sender }
    }


    /// 在下面函数中，通过参数传入的 future 被 `Task` 包裹起来，然后会被推入到调度队列中，当 `run` 被调用时，该 future 将被执行
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Task::spawn(future, &self.sender);
    }
}


struct Delay {
    when: Instant,
}

impl Future for Delay {
    type Output = &'static str;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if Instant::now() >= self.when {
            println!("Hello world");
            Poll::Ready("done")
        } else {
            // 为当然任务克隆一个 waker 的句柄
            let waker = cx.waker().clone();
            let when = self.when;

            // 生成一个计时器线程
            thread::spawn(move || {
                let now = Instant::now();

                if  now < when {
                    thread::sleep(when - now);
                }

                waker.wake();
            });

            Poll::Pending
        }
    }
}